use crate::bridge::Bridge;
use crate::catalog::{AppRecord, Catalog};
use crate::environment::SharedEnvironment;
use crate::governor::win::{
    enumerate_and_attach_renderers, query_app_memory_async, query_job_stats, snapshot_processes,
    AppMemoryUsage, Governor, ProcessKind, ProcessSnapshot,
};
use crate::instance::{AppInstance, Lifecycle};
use crate::manifest;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{NewWindowResponse, PageLoadEvent, WebContext, WebView, WebViewBuilder};

#[derive(Debug, Clone)]
pub enum UserEvent {
    Launch(String),
    AddApp { url: String },
    ManifestIngested { app_id: String, name: Option<String>, icon_path: Option<String> },
    BridgeResponse { window_id: WindowId, script: String },

    OpenDashboard,
    DashboardRefresh,
    MemoryPollResult(Vec<AppMemoryUsage>),

    DashboardKill { app_id: String },
    DashboardLaunch { app_id: String },
    DashboardGrant { app_id: String, capability: String, scope: String },
    DashboardRevoke { app_id: String, capability: String, scope: String },
}

struct AppWindow {
    window: Window,
    webview: WebView,
    instance: AppInstance,
    started_at: Instant,
    peak_memory: u64,
}

struct DashboardWindow {
    _window: Window,
    webview: WebView,
    memory_cache: HashMap<String, u64>,
}

const LAUNCHER_HTML: &str = r##"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  :root {
    --bg: #0a0a0c; --panel: #121217; --border: #25252b; --border-hover: #3a3a42;
    --text: #ffffff; --text-muted: #8a8a93; --accent: #0084ff; --accent-hover: #2a9bff;
  }
  * { margin:0; padding:0; box-sizing:border-box; }
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif; background: var(--bg); color: var(--text); padding: 40px 48px; }
  .header { display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 32px; }
  .brand { display: flex; align-items: center; }
  .brand-text h1 { font-size: 32px; font-weight: 700; letter-spacing: -0.5px; margin-bottom: 4px; color: var(--text); }
  .brand-text p { font-size: 14px; color: var(--text-muted); }
  .btn-dashboard { display: flex; align-items: center; gap: 8px; background: var(--panel); border: 1px solid var(--border); padding: 10px 16px; border-radius: 8px; color: var(--text); font-size: 14px; font-weight: 500; cursor: pointer; transition: all 0.2s; font-family: inherit; }
  .btn-dashboard:hover { background: var(--border); border-color: var(--border-hover); }
  .btn-dashboard svg { width: 16px; height: 16px; color: var(--accent); }
  
  /* Add URL Form */
  .search-container { display: flex; align-items: center; background: var(--panel); border: 1px solid var(--border); border-radius: 12px; padding: 6px; margin-bottom: 16px; transition: border-color 0.2s, box-shadow 0.2s; }
  .search-container:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px rgba(0, 132, 255, 0.15); }
  .search-icon { margin-left: 16px; margin-right: 12px; color: var(--text-muted); display: flex; align-items: center; }
  .search-icon svg { width: 20px; height: 20px; }
  .search-input { flex: 1; background: transparent; border: none; outline: none; color: var(--text); font-size: 16px; font-family: inherit; padding: 10px 0; }
  .search-input::placeholder { color: var(--text-muted); }
  .btn-add { background: var(--accent); color: #fff; border: none; padding: 10px 24px; border-radius: 8px; font-size: 15px; font-weight: 600; font-family: inherit; cursor: pointer; transition: background 0.2s; }
  .btn-add:hover { background: var(--accent-hover); }

  /* Filter Installed Apps */
  .filter-container { margin-bottom: 24px; }
  .filter-input { width: 100%; background: var(--panel); border: 1px solid var(--border); border-radius: 8px; padding: 12px 16px; color: var(--text); font-size: 14px; font-family: inherit; outline: none; transition: border-color 0.2s; }
  .filter-input:focus { border-color: var(--accent); }
  .filter-input::placeholder { color: var(--text-muted); }

  .notice { min-height: 20px; font-size: 13px; margin-bottom: 24px; text-align: center; opacity: 0; transition: opacity 0.2s; }
  .notice.active { opacity: 1; }
  .notice.ok { color: #34c759; }
  .notice.error { color: #ff453a; }
  
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 16px; }
  .app { background: var(--panel); border: 1px solid var(--border); border-radius: 12px; padding: 20px; cursor: pointer; transition: all 0.2s; display: flex; flex-direction: column; align-items: flex-start; position: relative; overflow: hidden; }
  .app:hover { background: #1a1a20; border-color: var(--border-hover); transform: translateY(-2px); }
  .app .icon { width: 40px; height: 40px; margin-bottom: 16px; border-radius: 10px; display: block; background: #25252b; object-fit: contain; }
  .app .name { font-size: 14px; font-weight: 600; margin-bottom: 4px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; width: 100%; }
  .app .url { font-size: 11px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; width: 100%; }
  
  .empty { color: var(--text-muted); font-size: 13px; padding: 40px 0; text-align: center; display: none; }
</style>
</head>
<body>
  <div class="header">
    <div class="brand"><div class="brand-text"><h1>Reeve</h1><p>One engine. Many apps. Governed.</p></div></div>
    <button class="btn-dashboard" onclick="openDashboard()">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7"></rect><rect x="14" y="3" width="7" height="7"></rect><rect x="14" y="14" width="7" height="7"></rect><rect x="3" y="14" width="7" height="7"></rect></svg>
      Dashboard
    </button>
  </div>
  
  <form class="search-container" id="add-form" onsubmit="return addApp(event)">
    <div class="search-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"></path><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"></path></svg></div>
    <input id="add-input" class="search-input" type="url" placeholder="Paste a PWA link (e.g. https://linear.app)" autocomplete="off" spellcheck="false" />
    <button class="btn-add" type="submit">Add</button>
  </form>
  
  <div class="notice" id="notice"></div>

  <div class="filter-container">
    <input id="filter-input" class="filter-input" type="text" placeholder="Search installed apps..." autocomplete="off" />
  </div>

  <div class="grid" id="grid"></div>
  <p class="empty" id="empty">No apps found.</p>

<script>
  window.__allApps = [];

  function launch(id) { window.ipc.postMessage(JSON.stringify({cmd:'launch', app_id:id})); }
  function openDashboard() { window.ipc.postMessage(JSON.stringify({cmd:'open_dashboard'})); }
  
  function addApp(e) {
    e.preventDefault();
    var input = document.getElementById('add-input');
    var url = input.value.trim();
    if (!url) return false;
    window.ipc.postMessage(JSON.stringify({ cmd: 'add_app', url: url }));
    input.value = '';
    return false;
  }
  
  function notice(msg, isError) {
    var el = document.getElementById('notice');
    el.textContent = msg;
    el.className = 'notice active ' + (isError ? 'error' : 'ok');
    clearTimeout(window.__noticeTimer);
    window.__noticeTimer = setTimeout(function() { el.className = 'notice'; }, 4000);
  }
  
  function hostOf(url) {
    try { return new URL(url).hostname; } catch (e) { return ''; }
  }
  
  function iconHTML(app) {
    if (app.icon_path) {
      var path = app.icon_path.replace(/\\/g, '/');
      if (!path.startsWith('/')) path = '/' + path;
      return '<img class="icon" alt="" src="file://' + encodeURI(path) + '">';
    }
    var host = hostOf(app.start_url);
    if (!host) return '<div class="icon"></div>';
    var src = 'https://www.google.com/s2/favicons?domain=' + encodeURIComponent(host) + '&sz=128';
    return '<img class="icon" alt="" src="' + src + '" onerror="this.style.visibility=\'hidden\'">';
  }

  function renderFiltered() {
    var query = document.getElementById('filter-input').value.toLowerCase();
    var filtered = window.__allApps.filter(function(app) {
      return app.name.toLowerCase().includes(query) || 
             app.start_url.toLowerCase().includes(query);
    });

    var grid = document.getElementById('grid');
    var empty = document.getElementById('empty');

    if (filtered.length === 0) {
      grid.innerHTML = '';
      empty.style.display = 'block';
      empty.textContent = query ? 'No installed apps match your search.' : 'No apps installed yet.';
      return;
    }

    empty.style.display = 'none';
    grid.innerHTML = filtered.map(function(a) {
      return '<div class="app" onclick="launch(\'' + a.id + '\')">' + 
        iconHTML(a) + 
        '<div class="name">' + a.name + '</div>' + 
        '<div class="url">' + a.start_url + '</div></div>';
    }).join('');
  }

  window.__reeve_render = function(apps) {
    window.__allApps = apps;
    renderFiltered();
  };
  
  window.__reeve_notice = notice;

  document.getElementById('filter-input').addEventListener('input', renderFiltered);
</script>
</body>
</html>"##;

const DASHBOARD_HTML: &str = r##"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  :root {
    --bg: #0a0a0c; --panel: #121217; --border: #25252b; --border-hover: #3a3a42;
    --text: #ffffff; --text-muted: #8a8a93; --accent: #0084ff; --accent-hover: #2a9bff;
  }
  * { margin:0; padding:0; box-sizing:border-box; }
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif; background: var(--bg); color: var(--text); padding: 40px 48px; }
  h1 { font-size: 32px; font-weight: 700; letter-spacing: -0.5px; margin-bottom: 4px; color: var(--text); }
  h2 { font-size: 16px; font-weight: 600; color: var(--text); margin: 32px 0 16px; }
  .header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 32px; }
  .sub { font-size: 14px; color: var(--text-muted); }
  .stats { display: grid; grid-template-columns: repeat(4, 1fr); gap: 16px; margin-bottom: 16px; }
  .stat { background: var(--panel); border: 1px solid var(--border); border-radius: 12px; padding: 20px; }
  .stat .label { font-size: 12px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 8px; font-weight: 600; }
  .stat .value { font-size: 28px; font-weight: 700; color: var(--text); }
  .stat .value.accent { color: var(--accent); }
  table { width: 100%; border-collapse: collapse; background: var(--panel); border-radius: 12px; overflow: hidden; border: 1px solid var(--border); }
  th { text-align: left; padding: 12px 20px; font-size: 12px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.5px; background: #18181d; font-weight: 600; }
  td { padding: 16px 20px; border-top: 1px solid var(--border); font-size: 14px; vertical-align: middle; }
  tr:hover td { background: #18181d; }
  .name { font-weight: 600; color: var(--text); }
  .url { font-size: 12px; color: var(--text-muted); margin-top: 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 280px; }
  .badge { display: inline-block; padding: 4px 10px; border-radius: 6px; font-size: 12px; font-weight: 600; }
  .badge-open { background: rgba(52, 199, 89, 0.15); color: #34c759; }
  .badge-closed { background: rgba(142, 142, 147, 0.15); color: var(--text-muted); }
  .perm-chip { display: inline-flex; align-items: center; gap: 8px; padding: 4px 8px 4px 10px; margin: 2px 4px 2px 0; background: #1c1c22; border: 1px solid var(--border); border-radius: 6px; font-size: 12px; color: #c8c8d0; }
  .perm-chip button { background: transparent; border: none; color: var(--text-muted); cursor: pointer; padding: 0; font-size: 16px; line-height: 1; transition: color 0.2s; }
  .perm-chip button:hover { color: #ff453a; }
  .muted { color: var(--text-muted); font-size: 13px; }
  .btn-sm { padding: 6px 12px; font-size: 13px; font-weight: 500; background: var(--panel); border: 1px solid var(--border); color: var(--text); border-radius: 8px; cursor: pointer; font-family: inherit; transition: all 0.2s; }
  .btn-sm:hover { background: var(--border); border-color: var(--border-hover); }
  .btn-sm.btn-primary { background: var(--accent); border-color: var(--accent); color: #fff; }
  .btn-sm.btn-primary:hover { background: var(--accent-hover); border-color: var(--accent-hover); }
  .btn-sm.btn-danger { background: rgba(255, 69, 58, 0.1); border-color: rgba(255, 69, 58, 0.2); color: #ff453a; }
  .btn-sm.btn-danger:hover { background: rgba(255, 69, 58, 0.2); border-color: rgba(255, 69, 58, 0.3); }
  .btn-sm.btn-add { padding: 4px 10px; font-size: 12px; }
  dialog { background: var(--panel); color: var(--text); border: 1px solid var(--border); border-radius: 12px; padding: 24px; max-width: 400px; width: 100%; }
  dialog::backdrop { background: rgba(0, 0, 0, 0.8); backdrop-filter: blur(4px); }
  dialog h3 { font-size: 18px; margin-bottom: 20px; color: var(--text); font-weight: 600; }
  dialog label { display: block; margin-bottom: 16px; font-size: 13px; color: var(--text-muted); }
  dialog input, dialog select { display: block; width: 100%; margin-top: 6px; background: var(--bg); border: 1px solid var(--border); border-radius: 8px; padding: 10px 12px; color: var(--text); font-size: 14px; font-family: inherit; outline: none; transition: border-color 0.2s; }
  dialog input:focus, dialog select:focus { border-color: var(--accent); }
  dialog .actions { display: flex; gap: 12px; justify-content: flex-end; margin-top: 24px; }
  .processes { font-size: 12px; color: var(--text-muted); margin-top: 8px; }
  .processes span { display: inline-block; margin-right: 12px; }
  .processes strong { color: var(--text); font-weight: 600; }
  .note { font-size: 11px; color: #5a5a66; margin-top: 6px; }
</style>
</head>
<body>
  <div class="header">
    <div><h1>Dashboard</h1><p class="sub">Live runtime, permissions, kill switch</p></div>
    <button onclick="refresh()" class="btn-sm">Refresh</button>
  </div>
  <section class="stats">
    <div class="stat">
      <div class="label">Total now</div>
      <div class="value" id="stat-memory">—</div>
      <div class="processes" id="stat-procs"></div>
    </div>
    <div class="stat">
      <div class="label">Peak job memory</div>
      <div class="value accent" id="stat-peak">—</div>
      <div class="note">Highest committed memory ever seen by the shared Job Object</div>
    </div>
    <div class="stat"><div class="label">Open apps</div><div class="value" id="stat-open">—</div></div>
    <div class="stat"><div class="label">Installed apps</div><div class="value" id="stat-installed">—</div></div>
  </section>
  <h2>Applications</h2>
  <table>
    <thead>
      <tr>
        <th>App</th><th>Status</th><th>Uptime</th><th>Memory</th><th>Peak (session)</th><th>Permissions</th><th>Actions</th>
      </tr>
    </thead>
    <tbody id="app-rows"></tbody>
  </table>
  <dialog id="perm-modal">
    <form onsubmit="return submitPerm(event)">
      <h3>Add permission</h3>
      <input type="hidden" id="perm-app-id">
      <label>Capability
        <select id="perm-cap"><option value="notify">notify</option><option value="fs.pick">fs.pick</option><option value="fs.read">fs.read</option><option value="fs.write">fs.write</option></select>
      </label>
      <label>Scope (path prefix, blank for none)
        <input id="perm-scope" type="text" placeholder="C:\Users\Public">
      </label>
      <div class="actions">
        <button type="button" class="btn-sm" onclick="document.getElementById('perm-modal').close()">Cancel</button>
        <button type="submit" class="btn-sm btn-primary">Add</button>
      </div>
    </form>
  </dialog>
<script>
  function fmtBytes(n) { if (!n) return "0 B"; var units = ["B","KB","MB","GB"]; var i = 0; while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; } return n.toFixed(1) + " " + units[i]; }
  function fmtUptime(s) { if (s < 60) return s + "s"; if (s < 3600) return Math.floor(s/60) + "m " + (s%60) + "s"; return Math.floor(s/3600) + "h " + Math.floor((s%3600)/60) + "m"; }
  function esc(s) { return String(s).replace(/[&<>"']/g, function(c) { return {'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]; }); }
  function refresh() { window.ipc.postMessage(JSON.stringify({cmd:'dashboard_refresh'})); }
  function killApp(id) { if (!confirm('Kill ' + id + '?')) return; window.ipc.postMessage(JSON.stringify({cmd:'dashboard_kill', app_id: id})); }
  function launchApp(id) { window.ipc.postMessage(JSON.stringify({cmd:'dashboard_launch', app_id: id})); }
  function revoke(appId, cap, scope) { window.ipc.postMessage(JSON.stringify({cmd:'dashboard_revoke', app_id: appId, capability: cap, scope: scope})); }
  function openPermModal(appId) { document.getElementById('perm-app-id').value = appId; document.getElementById('perm-scope').value = ''; document.getElementById('perm-cap').value = 'notify'; document.getElementById('perm-modal').showModal(); }
  function submitPerm(e) { e.preventDefault(); window.ipc.postMessage(JSON.stringify({ cmd:'dashboard_grant', app_id: document.getElementById('perm-app-id').value, capability: document.getElementById('perm-cap').value, scope: document.getElementById('perm-scope').value })); document.getElementById('perm-modal').close(); return false; }
  function render(data) {
    document.getElementById('stat-memory').textContent = fmtBytes(data.total_memory_bytes);
    document.getElementById('stat-peak').textContent = data.peak_job_memory != null ? fmtBytes(data.peak_job_memory) : '—';
    document.getElementById('stat-open').textContent = data.apps.filter(function(a) { return a.is_open; }).length;
    document.getElementById('stat-installed').textContent = data.apps.length;
    var procs = data.processes.map(function(p) { return '<span>' + p.kind + ' <strong>' + fmtBytes(p.memory_bytes || 0) + '</strong></span>'; }).join('');
    document.getElementById('stat-procs').innerHTML = procs || '<span class="muted">no processes</span>';
    var tbody = document.getElementById('app-rows');
    if (!data.apps.length) { tbody.innerHTML = '<tr><td colspan="7" class="muted" style="text-align:center; padding: 40px;">No apps installed</td></tr>'; return; }
    tbody.innerHTML = data.apps.map(function(app) {
      var status = app.is_open ? '<span class="badge badge-open">Open</span>' : '<span class="badge badge-closed">Closed</span>';
      var uptime = app.is_open && app.uptime_secs != null ? fmtUptime(app.uptime_secs) : '—';
      var mem = app.memory_bytes != null ? fmtBytes(app.memory_bytes) : '—';
      var peak = app.peak_session_bytes != null ? fmtBytes(app.peak_session_bytes) : '—';
      var permHtml = app.permissions.map(function(p) { var label = p.scope ? p.capability + ': ' + p.scope : p.capability; return '<span class="perm-chip">' + esc(label) + '<button title="Revoke" onclick="revoke(\'' + esc(app.id) + '\',\'' + esc(p.capability) + '\',\'' + esc(p.scope) + '\')">&times;</button></span>'; }).join('');
      permHtml += ' <button class="btn-sm btn-add" onclick="openPermModal(\'' + esc(app.id) + '\')">+ Add</button>';
      var action = app.is_open ? '<button class="btn-sm btn-danger" onclick="killApp(\'' + esc(app.id) + '\')">Kill</button>' : '<button class="btn-sm btn-primary" onclick="launchApp(\'' + esc(app.id) + '\')">Launch</button>';
      return '<tr><td><div class="name">' + esc(app.name) + '</div><div class="url">' + esc(app.url) + '</div></td><td>' + status + '</td><td>' + uptime + '</td><td>' + mem + '</td><td>' + peak + '</td><td>' + (permHtml || '<span class="muted">none</span>') + '</td><td>' + action + '</td></tr>';
    }).join('');
  }
  window.__reeve_dashboard = render;
  window.addEventListener('DOMContentLoaded', refresh);
</script>
</body>
</html>"##;

#[derive(Serialize)]
struct DashboardData {
    total_memory_bytes: u64,
    peak_job_memory: Option<u64>,
    process_count: u32,
    apps: Vec<DashboardApp>,
    processes: Vec<DashboardProcess>,
}

#[derive(Serialize)]
struct DashboardApp {
    id: String,
    name: String,
    url: String,
    is_open: bool,
    uptime_secs: Option<u64>,
    memory_bytes: Option<u64>,
    peak_session_bytes: Option<u64>,
    permissions: Vec<DashboardPermission>,
}

#[derive(Serialize)]
struct DashboardPermission {
    capability: String,
    scope: String,
}

#[derive(Serialize)]
struct DashboardProcess {
    pid: u32,
    kind: String,
    memory_bytes: Option<u64>,
}

pub fn run_launcher(catalog: Catalog, env: SharedEnvironment, governor: Governor) -> Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let apps = catalog.list_apps()?;
    let apps_json = serde_json::to_string(&apps)?;

    let launcher_window = WindowBuilder::new()
        .with_title("Reeve")
        .with_inner_size(tao::dpi::LogicalSize::new(800.0, 600.0))
        .build(&event_loop)?;

    let mut web_context = WebContext::new(Some(env.data_dir.clone()));

    let launcher_webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_html(LAUNCHER_HTML)
        .with_ipc_handler({
            let proxy = proxy.clone();
            move |req| {
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(req.body()) else { return; };
                match msg["cmd"].as_str() {
                    Some("launch") => { if let Some(app_id) = msg["app_id"].as_str() { let _ = proxy.send_event(UserEvent::Launch(app_id.to_string())); } }
                    Some("add_app") => { if let Some(url) = msg["url"].as_str() { let _ = proxy.send_event(UserEvent::AddApp { url: url.to_string() }); } }
                    Some("open_dashboard") => { let _ = proxy.send_event(UserEvent::OpenDashboard); }
                    _ => {}
                }
            }
        })
        .build(&launcher_window)?;

    launcher_webview.evaluate_script(&format!("window.__reeve_render({apps_json})"))?;

    let _launcher_window = launcher_window;
    let launcher_webview_opt = Some(launcher_webview);

    let mut app_windows: HashMap<WindowId, AppWindow> = HashMap::new();
    let mut dashboard: Option<DashboardWindow> = None;
    let governor = Rc::new(governor);
    let catalog = Rc::new(catalog);
    let data_dir = env.data_dir.clone();

    let mut last_scan = Instant::now();
    let scan_interval = Duration::from_secs(3);

    let mut last_dashboard_push = Instant::now();
    let dashboard_push_interval = Duration::from_secs(2);

    let mut last_memory_poll = Instant::now();
    let memory_poll_interval = Duration::from_secs(5);
    let mut memory_query_in_flight = false;
    let mut memory_query_started = Instant::now();

    if let Ok(auto) = std::env::var("REEVE_BENCH_AUTO_LAUNCH") {
        for url in auto.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if let Ok((id, name, canonical)) = derive_app_from_url(url) {
                let _ = catalog.add_app(&id, &name, &canonical);
                let _ = proxy.send_event(UserEvent::Launch(id));
            }
        }
        println!("REEVE_BENCH_READY");
    }

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500));

        match event {
            Event::UserEvent(UserEvent::Launch(app_id)) => {
                if let Some(aw) = app_windows.values().find(|aw| aw.instance.app.id == app_id) {
                    let _ = aw.window.set_focus();
                    return;
                }
                let app = match catalog.list_apps().unwrap_or_default().into_iter().find(|a| a.id == app_id) {
                    Some(a) => a,
                    None => return,
                };

                match spawn_app_window(event_loop, &mut web_context, app.clone(), proxy.clone(), catalog.clone()) {
                    Ok((window, webview, mut instance, started_at)) => {
                        instance.transition(Lifecycle::Ready);
                        tracing::info!(app_id = %app.id, session = instance.session_id, "app spawned");
                        app_windows.insert(window.id(), AppWindow { window, webview, instance, started_at, peak_memory: 0 });
                    }
                    Err(e) => tracing::error!(?e, %app_id, "failed to spawn app"),
                }
            }

            Event::UserEvent(UserEvent::AddApp { url }) => {
                match derive_app_from_url(&url) {
                    Ok((id, name, canonical)) => {
                        let already = catalog.list_apps().unwrap_or_default().iter().any(|a| a.id == id || a.start_url == canonical);
                        if already {
                            push_notice(&launcher_webview_opt, "Already installed", true);
                        } else if let Err(e) = catalog.add_app(&id, &name, &canonical) {
                            tracing::error!(?e, %id, "failed to add app");
                            push_notice(&launcher_webview_opt, "Failed to add app", true);
                        } else {
                            if let Some(apps) = catalog.list_apps().ok() {
                                if let Ok(json) = serde_json::to_string(&apps) {
                                    if let Some(wv) = launcher_webview_opt.as_ref() {
                                        let _ = wv.evaluate_script(&format!("window.__reeve_render({json})"));
                                    }
                                }
                            }
                            push_notice(&launcher_webview_opt, &format!("Added {name} (fetching icon...)"), false);

                            let proxy_clone = proxy.clone();
                            let data_clone = data_dir.clone();
                            let id_clone = id.clone();
                            let canonical_clone = canonical.clone();
                            std::thread::spawn(move || {
                                if let Ok(md) = manifest::fetch_manifest_data(&canonical_clone, &id_clone, &data_clone) {
                                    let _ = proxy_clone.send_event(UserEvent::ManifestIngested {
                                        app_id: id_clone,
                                        name: md.name,
                                        icon_path: md.icon_path.map(|p| p.to_string_lossy().to_string()),
                                    });
                                }
                            });
                        }
                    }
                    Err(e) => push_notice(&launcher_webview_opt, &format!("Invalid URL: {e}"), true),
                }
            }

            Event::UserEvent(UserEvent::ManifestIngested { app_id, name, icon_path }) => {
                let mut updated = false;
                if let Some(n) = name {
                    let _ = catalog.set_app_name(&app_id, &n);
                    updated = true;
                }
                if let Some(p) = icon_path {
                    let _ = catalog.set_icon_path(&app_id, &std::path::PathBuf::from(p));
                    updated = true;
                }
                if updated {
                    if let Some(apps) = catalog.list_apps().ok() {
                        if let Ok(json) = serde_json::to_string(&apps) {
                            if let Some(wv) = launcher_webview_opt.as_ref() {
                                let _ = wv.evaluate_script(&format!("window.__reeve_render({json})"));
                            }
                        }
                    }
                }
            }

            Event::UserEvent(UserEvent::OpenDashboard) => {
                if let Some(d) = &dashboard {
                    let _ = d._window.set_focus();
                    push_dashboard(d, &catalog, &app_windows, &governor);
                    return;
                }
                match spawn_dashboard_window(event_loop, &mut web_context, proxy.clone()) {
                    Ok(d) => {
                        tracing::info!("dashboard opened");
                        push_dashboard(&d, &catalog, &app_windows, &governor);
                        dashboard = Some(d);
                    }
                    Err(e) => tracing::error!(?e, "failed to open dashboard"),
                }
            }

            Event::UserEvent(UserEvent::DashboardRefresh) => {
                if let Some(d) = &dashboard {
                    push_dashboard(d, &catalog, &app_windows, &governor);
                }
            }

            Event::UserEvent(UserEvent::MemoryPollResult(usage)) => {
                memory_query_in_flight = false;

                let mut by_app: HashMap<String, u64> = HashMap::new();
                for u in &usage {
                    *by_app.entry(u.app_id.clone()).or_insert(0) += u.memory_bytes;
                }

                for aw in app_windows.values_mut() {
                    if let Some(&mem) = by_app.get(&aw.instance.app.id) {
                        if mem > aw.peak_memory {
                            aw.peak_memory = mem;
                        }
                    }
                }

                if let Some(d) = &mut dashboard {
                    d.memory_cache = by_app;
                    push_dashboard(d, &catalog, &app_windows, &governor);
                }
            }

            Event::UserEvent(UserEvent::DashboardKill { app_id }) => {
                let target = app_windows.iter().find(|(_, aw)| aw.instance.app.id == app_id).map(|(id, _)| *id);
                if let Some(window_id) = target {
                    if let Some(mut aw) = app_windows.remove(&window_id) {
                        aw.instance.transition(Lifecycle::Killed);
                        if aw.instance.session_id >= 0 {
                            if aw.peak_memory > 0 {
                                let _ = catalog.update_session_peak(aw.instance.session_id, aw.peak_memory);
                            }
                            if let Some(stats) = query_job_stats(&governor) {
                                tracing::info!(
                                    session_id = aw.instance.session_id,
                                    app_peak = aw.peak_memory,
                                    job_peak = stats.peak_job_memory,
                                    job_active = stats.active_processes,
                                    "session killed (peak cross-check)"
                                );
                            }
                            let _ = catalog.end_session(aw.instance.session_id);
                        }
                        tracing::info!(%app_id, "killed via dashboard");
                    }
                }
                if let Some(d) = &dashboard { push_dashboard(d, &catalog, &app_windows, &governor); }
            }

            Event::UserEvent(UserEvent::DashboardLaunch { app_id }) => {
                let _ = proxy.send_event(UserEvent::Launch(app_id));
            }

            Event::UserEvent(UserEvent::DashboardGrant { app_id, capability, scope }) => {
                if let Err(e) = catalog.grant_permission(&app_id, &capability, &scope) {
                    tracing::warn!(?e, %app_id, %capability, "grant failed");
                } else {
                    tracing::info!(%app_id, %capability, %scope, "permission granted");
                }
                if let Some(d) = &dashboard { push_dashboard(d, &catalog, &app_windows, &governor); }
            }

            Event::UserEvent(UserEvent::DashboardRevoke { app_id, capability, scope }) => {
                if let Err(e) = catalog.revoke_permission_scope(&app_id, &capability, &scope) {
                    tracing::warn!(?e, %app_id, %capability, "revoke failed");
                } else {
                    tracing::info!(%app_id, %capability, %scope, "permission revoked");
                }
                if let Some(d) = &dashboard { push_dashboard(d, &catalog, &app_windows, &governor); }
            }

            Event::UserEvent(UserEvent::BridgeResponse { window_id, script }) => {
                if let Some(aw) = app_windows.get(&window_id) {
                    if let Err(e) = aw.webview.evaluate_script(&script) {
                        tracing::warn!(?e, "failed to deliver bridge response");
                    }
                }
            }

            Event::WindowEvent { event: WindowEvent::Focused(focused), window_id, .. } => {
                if let Some(aw) = app_windows.get_mut(&window_id) {
                    let next = if focused { Lifecycle::Ready } else { Lifecycle::Backgrounded };
                    aw.instance.transition(next);
                }
            }

            Event::WindowEvent { event: WindowEvent::CloseRequested, window_id, .. } => {
                if let Some(mut aw) = app_windows.remove(&window_id) {
                    aw.instance.transition(Lifecycle::Killed);
                    if aw.instance.session_id >= 0 {
                        if aw.peak_memory > 0 {
                            let _ = catalog.update_session_peak(aw.instance.session_id, aw.peak_memory);
                        }
                        if let Some(stats) = query_job_stats(&governor) {
                            tracing::info!(
                                session_id = aw.instance.session_id,
                                app_peak = aw.peak_memory,
                                job_peak = stats.peak_job_memory,
                                job_active = stats.active_processes,
                                "session closed (peak cross-check)"
                            );
                        }
                        let _ = catalog.end_session(aw.instance.session_id);
                    }
                    tracing::info!(app_id = %aw.instance.app.id, "app closed");
                } else if dashboard.as_ref().map(|d| d._window.id()) == Some(window_id) {
                    tracing::info!("dashboard closed");
                    dashboard = None;
                } else {
                    *control_flow = ControlFlow::Exit;
                }
            }

            Event::LoopDestroyed => { tracing::info!("event loop destroyed"); }
            _ => {}
        }

        if last_scan.elapsed() >= scan_interval {
            last_scan = Instant::now();
            if let Some(aw) = app_windows.values().next() {
                match enumerate_and_attach_renderers(&governor, &aw.webview) {
                    Ok(pids) if !pids.is_empty() => { tracing::info!(?pids, "processes attached"); }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(?e, "renderer enumeration failed"),
                }
            }
        }

        if memory_query_in_flight && memory_query_started.elapsed() > Duration::from_secs(30) {
            tracing::warn!("memory poll timed out; resetting in-flight flag");
            memory_query_in_flight = false;
        }
        if !memory_query_in_flight && last_memory_poll.elapsed() >= memory_poll_interval {
            let anchor = launcher_webview_opt.as_ref()
                .or_else(|| dashboard.as_ref().map(|d| &d.webview))
                .or_else(|| app_windows.values().next().map(|aw| &aw.webview));

            if let Some(wv) = anchor {
                last_memory_poll = Instant::now();
                memory_query_started = Instant::now();
                memory_query_in_flight = true;
                let app_urls: Vec<(String, String)> = catalog.list_apps().unwrap_or_default()
                    .into_iter().map(|a| (a.id, a.start_url)).collect();
                let proxy_clone = proxy.clone();
                if let Err(e) = query_app_memory_async(wv, app_urls, Box::new(move |usage| {
                    let _ = proxy_clone.send_event(UserEvent::MemoryPollResult(usage));
                })) {
                    memory_query_in_flight = false;
                    tracing::warn!(?e, "memory poll failed to start");
                }
            }
        }

        if let Some(d) = &dashboard {
            if last_dashboard_push.elapsed() >= dashboard_push_interval {
                last_dashboard_push = Instant::now();
                push_dashboard(d, &catalog, &app_windows, &governor);
            }
        }
    });
}

fn spawn_dashboard_window(
    event_loop: &EventLoopWindowTarget<UserEvent>,
    web_context: &mut WebContext,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<DashboardWindow> {
    let window = WindowBuilder::new()
        .with_title("Reeve — Dashboard")
        .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 750.0))
        .build(event_loop)?;

    let webview = WebViewBuilder::new_with_web_context(web_context)
        .with_html(DASHBOARD_HTML)
        .with_ipc_handler(move |req| {
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(req.body()) else { return; };
            match msg["cmd"].as_str() {
                Some("dashboard_refresh") => { let _ = proxy.send_event(UserEvent::DashboardRefresh); }
                Some("dashboard_kill") => { if let Some(id) = msg["app_id"].as_str() { let _ = proxy.send_event(UserEvent::DashboardKill { app_id: id.to_string() }); } }
                Some("dashboard_launch") => { if let Some(id) = msg["app_id"].as_str() { let _ = proxy.send_event(UserEvent::DashboardLaunch { app_id: id.to_string() }); } }
                Some("dashboard_grant") => { if let (Some(id), Some(cap)) = (msg["app_id"].as_str(), msg["capability"].as_str()) { let scope = msg["scope"].as_str().unwrap_or(""); let _ = proxy.send_event(UserEvent::DashboardGrant { app_id: id.to_string(), capability: cap.to_string(), scope: scope.to_string() }); } }
                Some("dashboard_revoke") => { if let (Some(id), Some(cap)) = (msg["app_id"].as_str(), msg["capability"].as_str()) { let scope = msg["scope"].as_str().unwrap_or(""); let _ = proxy.send_event(UserEvent::DashboardRevoke { app_id: id.to_string(), capability: cap.to_string(), scope: scope.to_string() }); } }
                _ => {}
            }
        })
        .build(&window)?;

    Ok(DashboardWindow { _window: window, webview, memory_cache: HashMap::new() })
}

fn push_dashboard(
    dash: &DashboardWindow,
    catalog: &Catalog,
    app_windows: &HashMap<WindowId, AppWindow>,
    governor: &Governor,
) {
    let processes: Vec<ProcessSnapshot> = snapshot_processes(&dash.webview).unwrap_or_default();
    let apps = catalog.list_apps().unwrap_or_default();
    let now = Instant::now();

    let dashboard_apps: Vec<DashboardApp> = apps.into_iter().map(|app| {
        let open = app_windows.values().find(|aw| aw.instance.app.id == app.id);
        let (is_open, uptime_secs, peak_session_bytes) = match open {
            Some(aw) => (
                true,
                Some(now.duration_since(aw.started_at).as_secs()),
                if aw.peak_memory > 0 { Some(aw.peak_memory) } else { None },
            ),
            None => (false, None, None),
        };
        let permissions = catalog.get_all_permissions(&app.id).unwrap_or_default()
            .into_iter().map(|(capability, scope)| DashboardPermission { capability, scope }).collect();
        DashboardApp {
            id: app.id.clone(),
            name: app.name,
            url: app.start_url,
            is_open,
            uptime_secs,
            memory_bytes: dash.memory_cache.get(&app.id).copied(),
            peak_session_bytes,
            permissions,
        }
    }).collect();

    let total_memory_bytes: u64 = processes.iter().filter_map(|p| p.memory_bytes).sum();
    let process_count = processes.len() as u32;
    let peak_job_memory = query_job_stats(governor).map(|s| s.peak_job_memory);

    let dashboard_processes: Vec<DashboardProcess> = processes.iter().map(|p| DashboardProcess {
        pid: p.pid,
        kind: match p.kind {
            ProcessKind::Browser => "browser".into(),
            ProcessKind::Renderer => "renderer".into(),
            ProcessKind::Gpu => "gpu".into(),
            ProcessKind::Utility => "utility".into(),
            ProcessKind::Other => "other".into(),
        },
        memory_bytes: p.memory_bytes,
    }).collect();

    let data = DashboardData {
        total_memory_bytes,
        peak_job_memory,
        process_count,
        apps: dashboard_apps,
        processes: dashboard_processes,
    };

    if let Ok(json) = serde_json::to_string(&data) {
        let _ = dash.webview.evaluate_script(&format!("window.__reeve_dashboard({json})"));
    }
}

fn derive_app_from_url(url_str: &str) -> Result<(String, String, String)> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() { return Err(anyhow!("empty URL")); }
    let with_scheme = if trimmed.contains("://") { trimmed.to_string() } else { format!("https://{trimmed}") };
    let url = url::Url::parse(&with_scheme).map_err(|e| anyhow!("invalid URL: {e}"))?;
    if url.scheme() != "https" && url.scheme() != "http" { return Err(anyhow!("only http(s) URLs are supported")); }
    let host_raw = url.host_str().ok_or_else(|| anyhow!("URL has no host"))?;
    let host = host_raw.trim_start_matches("www.").trim_end_matches('.').to_lowercase();
    if host.is_empty() { return Err(anyhow!("URL has no host")); }
    let id = host.replace('.', "-");
    let name = derive_display_name(&host);
    let mut canonical = format!("{}://{}", url.scheme(), host);
    if let Some(port) = url.port() { canonical.push_str(&format!(":{port}")); }
    let path = url.path();
    if !path.is_empty() && path != "/" { canonical.push_str(path); }
    Ok((id, name, canonical))
}

fn derive_display_name(host: &str) -> String {
    const SPECIAL: &[(&str, &str)] = &[("whatsapp", "WhatsApp"), ("youtube", "YouTube"), ("github", "GitHub"), ("gitlab", "GitLab"), ("vscode", "VS Code"), ("gmail", "Gmail")];
    let base = extract_brand_label(host);
    for (key, display) in SPECIAL { if base.eq_ignore_ascii_case(key) { return display.to_string(); } }
    capitalize(&base)
}

fn extract_brand_label(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return host.to_string(); }
    if parts.len() == 1 { return parts[0].to_string(); }
    const GENERIC_SL: &[&str] = &["co", "com", "org", "net", "gov", "edu", "ac", "ne", "or"];
    let mut idx = parts.len() - 2;
    while idx > 0 && GENERIC_SL.contains(&parts[idx]) { idx -= 1; }
    parts[idx].to_string()
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() { Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(), None => String::new() }
}

fn push_notice(webview: &Option<WebView>, msg: &str, is_error: bool) {
    if let Some(wv) = webview {
        let escaped = msg.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let flag = if is_error { "true" } else { "false" };
        let _ = wv.evaluate_script(&format!("window.__reeve_notice('{escaped}', {flag})"));
    }
}

enum PopupDisposition { Managed, SystemBrowser }

fn classify_popup(url: &str, parent_origin: Option<&str>) -> PopupDisposition {
    if url.is_empty() { return PopupDisposition::Managed; }
    let parsed = match url::Url::parse(url) { Ok(u) => u, Err(_) => return PopupDisposition::SystemBrowser };
    let scheme = parsed.scheme();
    if scheme == "about" || scheme == "blob" || scheme == "data" { return PopupDisposition::Managed; }
    if scheme != "https" && scheme != "http" { return PopupDisposition::SystemBrowser; }
    let host = match parsed.host_str() { Some(h) => h.trim_end_matches('.').to_lowercase(), None => return PopupDisposition::SystemBrowser };
    if let Some(expected) = parent_origin {
        let expected = expected.trim_end_matches('.').to_lowercase();
        if !expected.is_empty() && (host == expected || host.ends_with(&format!(".{expected}"))) { return PopupDisposition::Managed; }
        if same_brand_domain(&host, &expected) { return PopupDisposition::Managed; }
    }
    const OAUTH_HOSTS: &[&str] = &[
        "accounts.google.com", "ogs.google.com", "apis.google.com", "login.microsoftonline.com", "login.live.com", "login.windows.net",
        "microsoftonline.com", "office.com", "appleid.apple.com", "idmsa.apple.com", "auth0.com", "okta.com",
        "github.com", "githubusercontent.com", "slack.com", "slack-edge.com", "discord.com", "discordapp.com",
        "spotify.com", "accounts.spotify.com", "facebook.com", "twitter.com", "x.com", "linkedin.com", "notion.so", "notion.com", "linear.app", "figma.com",
    ];
    for allowed in OAUTH_HOSTS { if host == *allowed || host.ends_with(&format!(".{allowed}")) { return PopupDisposition::Managed; } }
    if looks_like_oauth(&parsed) { return PopupDisposition::Managed; }
    PopupDisposition::SystemBrowser
}

fn same_brand_domain(host: &str, parent: &str) -> bool {
    let h = host.split('.').rev().nth(1).unwrap_or("").to_lowercase();
    let p = parent.split('.').rev().nth(1).unwrap_or("").to_lowercase();
    !h.is_empty() && !p.is_empty() && h != "co" && h != "com" && h == p
}

fn looks_like_oauth(url: &url::Url) -> bool {
    let query = match url.query() { Some(q) => q.to_lowercase(), None => return false };
    let has_redirect = query.contains("redirect_uri=") || query.contains("redirecturi=") || query.contains("redirecturl=") || query.contains("redirect_url=") || query.contains("redirecttoauth=");
    let has_client_id = query.contains("client_id=") || query.contains("clientid=");
    let has_response_type = query.contains("response_type=") || query.contains("responsetype=");
    let has_scope = query.contains("scope=");
    let has_state = query.contains("state=");
    let has_popup_marker = query.contains("callbacktype=popup") || query.contains("popupflowid=") || query.contains("popup=true") || query.contains("popup_type=");
    if has_popup_marker { return true; }
    if has_client_id && has_redirect { return true; }
    if has_response_type && has_scope { return true; }
    if has_state && (has_client_id || has_response_type) { return true; }
    has_redirect
}

fn spawn_app_window(
    event_loop: &EventLoopWindowTarget<UserEvent>,
    web_context: &mut WebContext,
    app: AppRecord,
    proxy: EventLoopProxy<UserEvent>,
    catalog: Rc<Catalog>,
) -> Result<(Window, WebView, AppInstance, Instant)> {
    let window = WindowBuilder::new()
        .with_title(app.name.clone())
        .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 750.0))
        .build(event_loop)?;
    let window_id = window.id();

    let mut permissions: HashMap<String, Vec<String>> = HashMap::new();
    for (cap, scope) in catalog.get_all_permissions(&app.id).unwrap_or_default() {
        permissions.entry(cap).or_default().push(scope);
    }
    let bridge = Bridge::new(app.id.clone(), permissions);

    let expected_host = url::Url::parse(&app.start_url).ok().and_then(|u| u.host_str().map(|s| s.to_string()));
    let session_id = catalog.record_session_start(&app.id, 0).unwrap_or(-1);
    let instance = AppInstance::spawn(app.clone(), session_id)?;
    let started_at = Instant::now();

    let bridge_host = expected_host.clone();
    let bridge_proxy = proxy.clone();
    let popup_parent_origin = expected_host.clone();
    let start_url = app.start_url.clone();
    let app_id_for_cb = app.id.clone();

    let webview = WebViewBuilder::new_with_web_context(web_context)
        .with_url(&start_url)
        .with_on_page_load_handler(move |event, _url| {
            if matches!(event, PageLoadEvent::Finished) {
                if std::env::var("REEVE_BENCH_AUTO_LAUNCH").is_ok() {
                    println!("REEVE_APP_READY:{}", app_id_for_cb);
                }
            }
        })
        .with_new_window_req_handler(move |popup_url, _features| {
            match classify_popup(&popup_url, popup_parent_origin.as_deref()) {
                PopupDisposition::Managed => NewWindowResponse::Allow,
                PopupDisposition::SystemBrowser => {
                    if let Err(e) = open::that(&popup_url) { tracing::warn!(?e, %popup_url, "failed to open system browser"); }
                    NewWindowResponse::Deny
                }
            }
        })
        .with_ipc_handler(move |req| {
            let page_host = req.uri().host();
            let origin_ok = match (&bridge_host, page_host) {
                (Some(expected), Some(host)) => host == expected || host.ends_with(&format!(".{expected}")),
                _ => false,
            };
            if !origin_ok { return; }
            let response_json = bridge.handle(req.body());
            let script = format!("window.dispatchEvent(new CustomEvent('reeve-response', {{ detail: {response_json} }}));");
            let _ = bridge_proxy.send_event(UserEvent::BridgeResponse { window_id, script });
        })
        .build(&window)?;

    Ok((window, webview, instance, started_at))
}