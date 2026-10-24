use crate::bridge::Bridge;
use crate::catalog::{AppRecord, Catalog};
use crate::environment::SharedEnvironment;
use crate::governor::win::{
    enumerate_and_attach_renderers, snapshot_processes, Governor, ProcessKind, ProcessSnapshot,
};
use crate::instance::{AppInstance, Lifecycle};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder};

#[derive(Debug, Clone)]
pub enum UserEvent {
    Launch(String),
    AddApp { url: String },
    BridgeResponse { window_id: WindowId, script: String },

    OpenDashboard,
    DashboardRefresh,
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
}

struct DashboardWindow {
    _window: Window,
    webview: WebView,
}

const LAUNCHER_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  * { margin:0; padding:0; box-sizing:border-box; }
  body { font-family: 'Segoe UI', system-ui, sans-serif; background:#0f0f11; color:#e8e8ec; padding:24px; }
  .header { display:flex; align-items:center; justify-content:space-between; margin-bottom:16px; }
  h1 { font-size:18px; font-weight:600; color:#fff; }
  .sub { font-size:12px; color:#6b6b76; margin-top:4px; }
  .header-actions { display:flex; gap:8px; }
  form { display:flex; gap:8px; margin-bottom:12px; }
  input[type=url] {
    flex:1; background:#1a1a1f; border:1px solid #2a2a32; border-radius:8px;
    padding:10px 12px; color:#e8e8ec; font-size:13px; outline:none;
    font-family: inherit;
  }
  input[type=url]:focus { border-color:#4a4a5a; }
  input[type=url]::placeholder { color:#5a5a66; }
  button {
    background:#2a2a32; border:1px solid #3a3a44; color:#e8e8ec;
    padding:10px 16px; border-radius:8px; font-size:13px;
    cursor:pointer; font-family: inherit;
  }
  button:hover { background:#3a3a44; }
  .notice { min-height:20px; font-size:12px; margin-bottom:12px; }
  .notice.ok { color:#5dc26a; }
  .notice.error { color:#e05050; }
  .grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(140px,1fr)); gap:12px; }
  .app {
    background:#1a1a1f; border:1px solid #2a2a32; border-radius:10px;
    padding:16px; cursor:pointer; transition:all .15s;
    display:flex; flex-direction:column; align-items:flex-start;
  }
  .app:hover { border-color:#4a4a5a; background:#202028; }
  .app .icon {
    width:32px; height:32px; margin-bottom:10px;
    border-radius:6px; display:block;
    background:#2a2a32; object-fit:contain;
  }
  .app .name { font-size:13px; font-weight:500; }
  .app .url { font-size:10px; color:#5a5a66; margin-top:4px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; max-width:100%; }
  .empty { color:#5a5a66; font-size:13px; padding:40px 0; text-align:center; }
</style>
</head>
<body>
  <div class="header">
    <div>
      <h1>Reeve</h1>
      <p class="sub">One engine. Many apps. Governed.</p>
    </div>
    <div class="header-actions">
      <button onclick="openDashboard()">Dashboard</button>
    </div>
  </div>
  <form id="add-form" onsubmit="return addApp(event)">
    <input id="add-input" type="url" placeholder="Paste a PWA link (e.g. https://linear.app)" autocomplete="off" spellcheck="false" />
    <button type="submit">Add</button>
  </form>
  <div id="notice" class="notice"></div>
  <div class="grid" id="grid"></div>
  <p class="empty" id="empty" style="display:none">No apps installed yet.</p>
<script>
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
    el.className = 'notice ' + (isError ? 'error' : 'ok');
    clearTimeout(window.__noticeTimer);
    window.__noticeTimer = setTimeout(function() {
      el.textContent = ''; el.className = 'notice';
    }, 4000);
  }
  function hostOf(url) {
    try { return new URL(url).hostname; } catch (e) { return ''; }
  }
  function iconHTML(url) {
    var host = hostOf(url);
    if (!host) return '<div class="icon"></div>';
    var src = 'https://www.google.com/s2/favicons?domain=' +
              encodeURIComponent(host) + '&sz=64';
    return '<img class="icon" alt="" src="' + src +
           '" onerror="this.style.visibility=\'hidden\'">';
  }
  function render(apps) {
    var grid = document.getElementById('grid');
    var empty = document.getElementById('empty');
    if (!apps.length) { empty.style.display='block'; return; }
    empty.style.display='none';
    grid.innerHTML = apps.map(function(a) {
      return '<div class="app" onclick="launch(\'' + a.id + '\')">' +
        iconHTML(a.start_url) +
        '<div class="name">' + a.name + '</div>' +
        '<div class="url">' + a.start_url + '</div></div>';
    }).join('');
  }
  window.__reeve_render = render;
  window.__reeve_notice = notice;
</script>
</body>
</html>"#;

const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  * { margin:0; padding:0; box-sizing:border-box; }
  body { font-family: 'Segoe UI', system-ui, sans-serif; background:#0f0f11; color:#e8e8ec; padding:24px; }
  h1 { font-size:18px; font-weight:600; color:#fff; }
  h2 { font-size:14px; font-weight:600; color:#fff; margin:20px 0 10px; }
  .header { display:flex; align-items:center; justify-content:space-between; margin-bottom:20px; }
  .sub { font-size:12px; color:#6b6b76; margin-top:4px; }
  .stats { display:grid; grid-template-columns:repeat(3,1fr); gap:12px; margin-bottom:8px; }
  .stat { background:#1a1a1f; border:1px solid #2a2a32; border-radius:10px; padding:16px; }
  .stat .label { font-size:11px; color:#6b6b76; text-transform:uppercase; letter-spacing:0.5px; margin-bottom:6px; }
  .stat .value { font-size:22px; font-weight:600; color:#fff; }
  table { width:100%; border-collapse:collapse; background:#1a1a1f; border-radius:10px; overflow:hidden; }
  th { text-align:left; padding:10px 14px; font-size:11px; color:#6b6b76; text-transform:uppercase; letter-spacing:0.5px; background:#15151a; }
  td { padding:12px 14px; border-top:1px solid #2a2a32; font-size:13px; vertical-align:middle; }
  tr:hover td { background:#1f1f25; }
  .name { font-weight:500; color:#fff; }
  .url { font-size:11px; color:#5a5a66; margin-top:2px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; max-width:280px; }
  .badge { display:inline-block; padding:3px 8px; border-radius:5px; font-size:11px; font-weight:500; }
  .badge-open { background:#1e3a24; color:#5dc26a; }
  .badge-closed { background:#2a2a32; color:#9090a0; }
  .perm-chip {
    display:inline-flex; align-items:center; gap:6px;
    padding:3px 6px 3px 8px; margin:2px 3px 2px 0;
    background:#252530; border:1px solid #353540; border-radius:5px;
    font-size:11px; color:#c8c8d0;
  }
  .perm-chip button {
    background:transparent; border:none; color:#707080; cursor:pointer;
    padding:0; font-size:14px; line-height:1; padding:0 2px;
  }
  .perm-chip button:hover { color:#e05050; }
  .muted { color:#5a5a66; font-size:12px; }
  .btn-sm {
    padding:5px 10px; font-size:12px;
    background:#2a2a32; border:1px solid #3a3a44; color:#e8e8ec;
    border-radius:6px; cursor:pointer; font-family:inherit;
  }
  .btn-sm:hover { background:#3a3a44; }
  .btn-sm.btn-primary { background:#2a3a52; border-color:#3a4a62; color:#8ab4f8; }
  .btn-sm.btn-primary:hover { background:#3a4a62; }
  .btn-sm.btn-danger { background:#3a2020; border-color:#4a2a2a; color:#e05050; }
  .btn-sm.btn-danger:hover { background:#4a2a2a; }
  .btn-sm.btn-add { padding:3px 8px; font-size:11px; }
  dialog {
    background:#1a1a1f; color:#e8e8ec; border:1px solid #3a3a44;
    border-radius:10px; padding:20px; max-width:400px;
  }
  dialog::backdrop { background:rgba(0,0,0,0.7); }
  dialog h3 { font-size:14px; margin-bottom:14px; color:#fff; }
  dialog label { display:block; margin-bottom:12px; font-size:12px; color:#a0a0b0; }
  dialog input, dialog select {
    display:block; width:100%; margin-top:4px;
    background:#0f0f11; border:1px solid #2a2a32; border-radius:6px;
    padding:8px 10px; color:#e8e8ec; font-size:13px; font-family:inherit;
    outline:none;
  }
  dialog input:focus, dialog select:focus { border-color:#4a4a5a; }
  dialog .actions { display:flex; gap:8px; justify-content:flex-end; margin-top:8px; }
  .processes { font-size:11px; color:#6b6b76; margin-top:8px; }
  .processes span { display:inline-block; margin-right:10px; }
</style>
</head>
<body>
  <div class="header">
    <div>
      <h1>Dashboard</h1>
      <p class="sub">Live runtime, permissions, kill switch</p>
    </div>
    <button onclick="refresh()" class="btn-sm">Refresh</button>
  </div>

  <section class="stats">
    <div class="stat">
      <div class="label">Total WebView2 memory</div>
      <div class="value" id="stat-memory">—</div>
      <div class="processes" id="stat-procs"></div>
    </div>
    <div class="stat">
      <div class="label">Open apps</div>
      <div class="value" id="stat-open">—</div>
    </div>
    <div class="stat">
      <div class="label">Installed apps</div>
      <div class="value" id="stat-installed">—</div>
    </div>
  </section>

  <h2>Applications</h2>
  <table>
    <thead>
      <tr>
        <th>App</th>
        <th>Status</th>
        <th>Uptime</th>
        <th>Permissions</th>
        <th>Actions</th>
      </tr>
    </thead>
    <tbody id="app-rows"></tbody>
  </table>

  <dialog id="perm-modal">
    <form onsubmit="return submitPerm(event)">
      <h3>Add permission</h3>
      <input type="hidden" id="perm-app-id">
      <label>Capability
        <select id="perm-cap">
          <option value="notify">notify</option>
          <option value="fs.pick">fs.pick</option>
          <option value="fs.read">fs.read</option>
          <option value="fs.write">fs.write</option>
        </select>
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
  function fmtBytes(n) {
    if (!n) return "0 B";
    var units = ["B","KB","MB","GB"];
    var i = 0;
    while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
    return n.toFixed(1) + " " + units[i];
  }
  function fmtUptime(s) {
    if (s < 60) return s + "s";
    if (s < 3600) return Math.floor(s/60) + "m " + (s%60) + "s";
    return Math.floor(s/3600) + "h " + Math.floor((s%3600)/60) + "m";
  }
  function esc(s) {
    return String(s).replace(/[&<>"']/g, function(c) {
      return {'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c];
    });
  }
  function refresh() {
    window.ipc.postMessage(JSON.stringify({cmd:'dashboard_refresh'}));
  }
  function killApp(id) {
    if (!confirm('Kill ' + id + '?')) return;
    window.ipc.postMessage(JSON.stringify({cmd:'dashboard_kill', app_id: id}));
  }
  function launchApp(id) {
    window.ipc.postMessage(JSON.stringify({cmd:'dashboard_launch', app_id: id}));
  }
  function revoke(appId, cap, scope) {
    window.ipc.postMessage(JSON.stringify({
      cmd:'dashboard_revoke', app_id: appId, capability: cap, scope: scope
    }));
  }
  function openPermModal(appId) {
    document.getElementById('perm-app-id').value = appId;
    document.getElementById('perm-scope').value = '';
    document.getElementById('perm-cap').value = 'notify';
    document.getElementById('perm-modal').showModal();
  }
  function submitPerm(e) {
    e.preventDefault();
    window.ipc.postMessage(JSON.stringify({
      cmd:'dashboard_grant',
      app_id: document.getElementById('perm-app-id').value,
      capability: document.getElementById('perm-cap').value,
      scope: document.getElementById('perm-scope').value
    }));
    document.getElementById('perm-modal').close();
    return false;
  }
  function render(data) {
    document.getElementById('stat-memory').textContent = fmtBytes(data.total_memory_bytes);
    document.getElementById('stat-open').textContent = data.apps.filter(function(a) { return a.is_open; }).length;
    document.getElementById('stat-installed').textContent = data.apps.length;

    var procs = data.processes.map(function(p) {
      return '<span>' + p.kind + ' <strong>' + fmtBytes(p.memory_bytes || 0) + '</strong></span>';
    }).join('');
    document.getElementById('stat-procs').innerHTML = procs || '<span class="muted">no processes</span>';

    var tbody = document.getElementById('app-rows');
    if (!data.apps.length) {
      tbody.innerHTML = '<tr><td colspan="5" class="muted" style="text-align:center">No apps installed</td></tr>';
      return;
    }
    tbody.innerHTML = data.apps.map(function(app) {
      var status = app.is_open
        ? '<span class="badge badge-open">Open</span>'
        : '<span class="badge badge-closed">Closed</span>';
      var uptime = app.is_open && app.uptime_secs != null
        ? fmtUptime(app.uptime_secs)
        : '—';

      var permHtml = app.permissions.map(function(p) {
        var label = p.scope ? p.capability + ': ' + p.scope : p.capability;
        return '<span class="perm-chip">' + esc(label) +
          '<button title="Revoke" onclick="revoke(\'' + esc(app.id) + '\',\'' +
          esc(p.capability) + '\',\'' + esc(p.scope) + '\')">&times;</button></span>';
      }).join('');
      permHtml += ' <button class="btn-sm btn-add" onclick="openPermModal(\'' + esc(app.id) + '\')">+ Add</button>';

      var action = app.is_open
        ? '<button class="btn-sm btn-danger" onclick="killApp(\'' + esc(app.id) + '\')">Kill</button>'
        : '<button class="btn-sm btn-primary" onclick="launchApp(\'' + esc(app.id) + '\')">Launch</button>';

      return '<tr>' +
        '<td><div class="name">' + esc(app.name) + '</div>' +
        '<div class="url">' + esc(app.url) + '</div></td>' +
        '<td>' + status + '</td>' +
        '<td>' + uptime + '</td>' +
        '<td>' + (permHtml || '<span class="muted">none</span>') + '</td>' +
        '<td>' + action + '</td>' +
        '</tr>';
    }).join('');
  }
  window.__reeve_dashboard = render;
  window.addEventListener('DOMContentLoaded', refresh);
</script>
</body>
</html>"#;

// --- dashboard data shape (serialized to JSON) ---

#[derive(Serialize)]
struct DashboardData {
    total_memory_bytes: u64,
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
        .with_inner_size(tao::dpi::LogicalSize::new(720.0, 560.0))
        .build(&event_loop)?;

    let mut web_context = WebContext::new(Some(env.data_dir.clone()));

    let launcher_webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_html(LAUNCHER_HTML)
        .with_ipc_handler({
            let proxy = proxy.clone();
            move |req| {
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(req.body()) else {
                    return;
                };
                match msg["cmd"].as_str() {
                    Some("launch") => {
                        if let Some(app_id) = msg["app_id"].as_str() {
                            let _ = proxy.send_event(UserEvent::Launch(app_id.to_string()));
                        }
                    }
                    Some("add_app") => {
                        if let Some(url) = msg["url"].as_str() {
                            let _ = proxy.send_event(UserEvent::AddApp {
                                url: url.to_string(),
                            });
                        }
                    }
                    Some("open_dashboard") => {
                        let _ = proxy.send_event(UserEvent::OpenDashboard);
                    }
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

    let mut last_scan = Instant::now();
    let scan_interval = std::time::Duration::from_secs(3);

    let mut last_dashboard_push = Instant::now();
    let dashboard_push_interval = std::time::Duration::from_secs(2);

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            Instant::now() + std::time::Duration::from_millis(500),
        );

        match event {
            Event::UserEvent(UserEvent::Launch(app_id)) => {
                if let Some(aw) = app_windows.values().find(|aw| aw.instance.app.id == app_id) {
                    let _ = aw.window.set_focus();
                    return;
                }
                let app = match catalog
                    .list_apps()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|a| a.id == app_id)
                {
                    Some(a) => a,
                    None => return,
                };

                match spawn_app_window(
                    event_loop,
                    &mut web_context,
                    app.clone(),
                    proxy.clone(),
                    catalog.clone(),
                ) {
                    Ok((window, webview, mut instance, started_at)) => {
                        instance.transition(Lifecycle::Ready);
                        tracing::info!(app_id = %app.id, session = instance.session_id, "app spawned");
                        app_windows.insert(
                            window.id(),
                            AppWindow { window, webview, instance, started_at },
                        );
                    }
                    Err(e) => tracing::error!(?e, %app_id, "failed to spawn app"),
                }
            }

            Event::UserEvent(UserEvent::AddApp { url }) => {
                match derive_app_from_url(&url) {
                    Ok((id, name, canonical)) => {
                        let already = catalog
                            .list_apps()
                            .unwrap_or_default()
                            .iter()
                            .any(|a| a.id == id || a.start_url == canonical);
                        if already {
                            push_notice(&launcher_webview_opt, "Already installed", true);
                        } else if let Err(e) = catalog.add_app(&id, &name, &canonical) {
                            tracing::error!(?e, %id, "failed to add app");
                            push_notice(&launcher_webview_opt, "Failed to add app", true);
                        } else {
                            if let Some(apps) = catalog.list_apps().ok() {
                                if let Ok(json) = serde_json::to_string(&apps) {
                                    if let Some(wv) = launcher_webview_opt.as_ref() {
                                        let _ = wv.evaluate_script(&format!(
                                            "window.__reeve_render({json})"
                                        ));
                                    }
                                }
                            }
                            push_notice(&launcher_webview_opt, &format!("Added {name}"), false);
                        }
                    }
                    Err(e) => push_notice(
                        &launcher_webview_opt,
                        &format!("Invalid URL: {e}"),
                        true,
                    ),
                }
            }

            Event::UserEvent(UserEvent::OpenDashboard) => {
                if let Some(d) = &dashboard {
                    let _ = d._window.set_focus();
                    push_dashboard(d, &catalog, &app_windows);
                    return;
                }
                match spawn_dashboard_window(event_loop, &mut web_context, proxy.clone()) {
                    Ok(d) => {
                        tracing::info!("dashboard opened");
                        push_dashboard(&d, &catalog, &app_windows);
                        dashboard = Some(d);
                    }
                    Err(e) => tracing::error!(?e, "failed to open dashboard"),
                }
            }

            Event::UserEvent(UserEvent::DashboardRefresh) => {
                if let Some(d) = &dashboard {
                    push_dashboard(d, &catalog, &app_windows);
                }
            }

            Event::UserEvent(UserEvent::DashboardKill { app_id }) => {
                let target = app_windows
                    .iter()
                    .find(|(_, aw)| aw.instance.app.id == app_id)
                    .map(|(id, _)| *id);
                if let Some(window_id) = target {
                    if let Some(mut aw) = app_windows.remove(&window_id) {
                        aw.instance.transition(Lifecycle::Killed);
                        if aw.instance.session_id >= 0 {
                            let _ = catalog.end_session(aw.instance.session_id);
                        }
                        tracing::info!(%app_id, "killed via dashboard");
                    }
                }
                if let Some(d) = &dashboard {
                    push_dashboard(d, &catalog, &app_windows);
                }
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
                if let Some(d) = &dashboard {
                    push_dashboard(d, &catalog, &app_windows);
                }
            }

            Event::UserEvent(UserEvent::DashboardRevoke { app_id, capability, scope }) => {
                if let Err(e) = catalog.revoke_permission_scope(&app_id, &capability, &scope) {
                    tracing::warn!(?e, %app_id, %capability, "revoke failed");
                } else {
                    tracing::info!(%app_id, %capability, %scope, "permission revoked");
                }
                if let Some(d) = &dashboard {
                    push_dashboard(d, &catalog, &app_windows);
                }
            }

            Event::UserEvent(UserEvent::BridgeResponse { window_id, script }) => {
                if let Some(aw) = app_windows.get(&window_id) {
                    if let Err(e) = aw.webview.evaluate_script(&script) {
                        tracing::warn!(?e, "failed to deliver bridge response");
                    }
                }
            }

            Event::WindowEvent {
                event: WindowEvent::Focused(focused),
                window_id,
                ..
            } => {
                if let Some(aw) = app_windows.get_mut(&window_id) {
                    let next = if focused {
                        Lifecycle::Ready
                    } else {
                        Lifecycle::Backgrounded
                    };
                    aw.instance.transition(next);
                }
            }

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } => {
                if let Some(mut aw) = app_windows.remove(&window_id) {
                    aw.instance.transition(Lifecycle::Killed);
                    if aw.instance.session_id >= 0 {
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

            Event::LoopDestroyed => {
                tracing::info!("event loop destroyed");
            }

            _ => {}
        }

        if last_scan.elapsed() >= scan_interval {
            last_scan = Instant::now();
            if let Some(aw) = app_windows.values().next() {
                match enumerate_and_attach_renderers(&governor, &aw.webview) {
                    Ok(pids) if !pids.is_empty() => {
                        tracing::info!(?pids, "processes attached");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(?e, "renderer enumeration failed"),
                }
            }
        }

        if let Some(d) = &dashboard {
            if last_dashboard_push.elapsed() >= dashboard_push_interval {
                last_dashboard_push = Instant::now();
                push_dashboard(d, &catalog, &app_windows);
            }
        }
    });
}

// --- dashboard helpers ---

fn spawn_dashboard_window(
    event_loop: &EventLoopWindowTarget<UserEvent>,
    web_context: &mut WebContext,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<DashboardWindow> {
    let window = WindowBuilder::new()
        .with_title("Reeve — Dashboard")
        .with_inner_size(tao::dpi::LogicalSize::new(960.0, 640.0))
        .build(event_loop)?;

    let webview = WebViewBuilder::new_with_web_context(web_context)
        .with_html(DASHBOARD_HTML)
        .with_ipc_handler(move |req| {
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(req.body()) else {
                return;
            };
            match msg["cmd"].as_str() {
                Some("dashboard_refresh") => {
                    let _ = proxy.send_event(UserEvent::DashboardRefresh);
                }
                Some("dashboard_kill") => {
                    if let Some(id) = msg["app_id"].as_str() {
                        let _ = proxy.send_event(UserEvent::DashboardKill {
                            app_id: id.to_string(),
                        });
                    }
                }
                Some("dashboard_launch") => {
                    if let Some(id) = msg["app_id"].as_str() {
                        let _ = proxy.send_event(UserEvent::DashboardLaunch {
                            app_id: id.to_string(),
                        });
                    }
                }
                Some("dashboard_grant") => {
                    if let (Some(id), Some(cap)) =
                        (msg["app_id"].as_str(), msg["capability"].as_str())
                    {
                        let scope = msg["scope"].as_str().unwrap_or("");
                        let _ = proxy.send_event(UserEvent::DashboardGrant {
                            app_id: id.to_string(),
                            capability: cap.to_string(),
                            scope: scope.to_string(),
                        });
                    }
                }
                Some("dashboard_revoke") => {
                    if let (Some(id), Some(cap)) =
                        (msg["app_id"].as_str(), msg["capability"].as_str())
                    {
                        let scope = msg["scope"].as_str().unwrap_or("");
                        let _ = proxy.send_event(UserEvent::DashboardRevoke {
                            app_id: id.to_string(),
                            capability: cap.to_string(),
                            scope: scope.to_string(),
                        });
                    }
                }
                _ => {}
            }
        })
        .build(&window)?;

    Ok(DashboardWindow { _window: window, webview })
}

fn push_dashboard(
    dash: &DashboardWindow,
    catalog: &Catalog,
    app_windows: &HashMap<WindowId, AppWindow>,
) {
    let processes: Vec<ProcessSnapshot> =
        snapshot_processes(&dash.webview).unwrap_or_default();

    let apps = catalog.list_apps().unwrap_or_default();
    let now = Instant::now();

    let dashboard_apps: Vec<DashboardApp> = apps
        .into_iter()
        .map(|app| {
            let open = app_windows.values().find(|aw| aw.instance.app.id == app.id);
            let (is_open, uptime_secs) = match open {
                Some(aw) => (true, Some(now.duration_since(aw.started_at).as_secs())),
                None => (false, None),
            };
            let permissions = catalog
                .get_all_permissions(&app.id)
                .unwrap_or_default()
                .into_iter()
                .map(|(capability, scope)| DashboardPermission { capability, scope })
                .collect();
            DashboardApp {
                id: app.id,
                name: app.name,
                url: app.start_url,
                is_open,
                uptime_secs,
                permissions,
            }
        })
        .collect();

    let total_memory_bytes: u64 = processes.iter().filter_map(|p| p.memory_bytes).sum();
    let process_count = processes.len() as u32;

    let dashboard_processes: Vec<DashboardProcess> = processes
        .iter()
        .map(|p| DashboardProcess {
            pid: p.pid,
            kind: match p.kind {
                ProcessKind::Browser => "browser".into(),
                ProcessKind::Renderer => "renderer".into(),
                ProcessKind::Gpu => "gpu".into(),
                ProcessKind::Utility => "utility".into(),
                ProcessKind::Other => "other".into(),
            },
            memory_bytes: p.memory_bytes,
        })
        .collect();

    let data = DashboardData {
        total_memory_bytes,
        process_count,
        apps: dashboard_apps,
        processes: dashboard_processes,
    };

    if let Ok(json) = serde_json::to_string(&data) {
        let _ = dash.webview.evaluate_script(&format!(
            "window.__reeve_dashboard({json})"
        ));
    }
}

// --- URL → app record derivation ---

fn derive_app_from_url(url_str: &str) -> Result<(String, String, String)> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("empty URL"));
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = url::Url::parse(&with_scheme).map_err(|e| anyhow!("invalid URL: {e}"))?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err(anyhow!("only http(s) URLs are supported"));
    }
    let host_raw = url.host_str().ok_or_else(|| anyhow!("URL has no host"))?;
    let host = host_raw
        .trim_start_matches("www.")
        .trim_end_matches('.')
        .to_lowercase();
    if host.is_empty() {
        return Err(anyhow!("URL has no host"));
    }
    let id = host.replace('.', "-");
    let name = derive_display_name(&host);
    let mut canonical = format!("{}://{}", url.scheme(), host);
    if let Some(port) = url.port() {
        canonical.push_str(&format!(":{port}"));
    }
    let path = url.path();
    if !path.is_empty() && path != "/" {
        canonical.push_str(path);
    }
    Ok((id, name, canonical))
}

fn derive_display_name(host: &str) -> String {
    const SPECIAL: &[(&str, &str)] = &[
        ("whatsapp", "WhatsApp"), ("youtube", "YouTube"),
        ("github", "GitHub"), ("gitlab", "GitLab"),
        ("vscode", "VS Code"), ("gmail", "Gmail"),
    ];
    let base = extract_brand_label(host);
    for (key, display) in SPECIAL {
        if base.eq_ignore_ascii_case(key) {
            return display.to_string();
        }
    }
    capitalize(&base)
}

fn extract_brand_label(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return host.to_string(); }
    if parts.len() == 1 { return parts[0].to_string(); }
    const GENERIC_SL: &[&str] = &["co", "com", "org", "net", "gov", "edu", "ac", "ne", "or"];
    let mut idx = parts.len() - 2;
    while idx > 0 && GENERIC_SL.contains(&parts[idx]) {
        idx -= 1;
    }
    parts[idx].to_string()
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn push_notice(webview: &Option<WebView>, msg: &str, is_error: bool) {
    if let Some(wv) = webview {
        let escaped = msg
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n");
        let flag = if is_error { "true" } else { "false" };
        let _ = wv.evaluate_script(&format!(
            "window.__reeve_notice('{escaped}', {flag})"
        ));
    }
}

// --- popup classification ---

enum PopupDisposition {
    Managed,
    SystemBrowser,
}

fn classify_popup(url: &str, parent_origin: Option<&str>) -> PopupDisposition {
    if url.is_empty() {
        return PopupDisposition::Managed;
    }
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return PopupDisposition::SystemBrowser,
    };
    let scheme = parsed.scheme();
    if scheme == "about" || scheme == "blob" || scheme == "data" {
        return PopupDisposition::Managed;
    }
    if scheme != "https" && scheme != "http" {
        return PopupDisposition::SystemBrowser;
    }
    let host = match parsed.host_str() {
        Some(h) => h.trim_end_matches('.').to_lowercase(),
        None => return PopupDisposition::SystemBrowser,
    };
    if let Some(expected) = parent_origin {
        let expected = expected.trim_end_matches('.').to_lowercase();
        if !expected.is_empty() && (host == expected || host.ends_with(&format!(".{expected}"))) {
            return PopupDisposition::Managed;
        }
        if same_brand_domain(&host, &expected) {
            return PopupDisposition::Managed;
        }
    }
    const OAUTH_HOSTS: &[&str] = &[
        "accounts.google.com", "ogs.google.com", "apis.google.com",
        "login.microsoftonline.com", "login.live.com", "login.windows.net",
        "microsoftonline.com", "office.com",
        "appleid.apple.com", "idmsa.apple.com",
        "auth0.com", "okta.com",
        "github.com", "githubusercontent.com",
        "slack.com", "slack-edge.com",
        "discord.com", "discordapp.com",
        "spotify.com", "accounts.spotify.com",
        "facebook.com", "twitter.com", "x.com", "linkedin.com",
        "notion.so", "notion.com", "linear.app", "figma.com",
    ];
    for allowed in OAUTH_HOSTS {
        if host == *allowed || host.ends_with(&format!(".{allowed}")) {
            return PopupDisposition::Managed;
        }
    }
    if looks_like_oauth(&parsed) {
        return PopupDisposition::Managed;
    }
    PopupDisposition::SystemBrowser
}

fn same_brand_domain(host: &str, parent: &str) -> bool {
    let h = host.split('.').rev().nth(1).unwrap_or("").to_lowercase();
    let p = parent.split('.').rev().nth(1).unwrap_or("").to_lowercase();
    !h.is_empty() && !p.is_empty() && h != "co" && h != "com" && h == p
}

fn looks_like_oauth(url: &url::Url) -> bool {
    let query = match url.query() {
        Some(q) => q.to_lowercase(),
        None => return false,
    };
    let has_redirect = query.contains("redirect_uri=")
        || query.contains("redirecturi=")
        || query.contains("redirecturl=")
        || query.contains("redirect_url=")
        || query.contains("redirecttoauth=");
    let has_client_id = query.contains("client_id=") || query.contains("clientid=");
    let has_response_type = query.contains("response_type=") || query.contains("responsetype=");
    let has_scope = query.contains("scope=");
    let has_state = query.contains("state=");
    let has_popup_marker = query.contains("callbacktype=popup")
        || query.contains("popupflowid=")
        || query.contains("popup=true")
        || query.contains("popup_type=");
    if has_popup_marker { return true; }
    if has_client_id && has_redirect { return true; }
    if has_response_type && has_scope { return true; }
    if has_state && (has_client_id || has_response_type) { return true; }
    has_redirect
}

// --- window construction ---

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

    let expected_host = url::Url::parse(&app.start_url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()));

    let session_id = catalog.record_session_start(&app.id, 0).unwrap_or(-1);
    let instance = AppInstance::spawn(app.clone(), session_id)?;
    let started_at = Instant::now();

    let bridge_host = expected_host.clone();
    let bridge_proxy = proxy.clone();
    let popup_parent_origin = expected_host.clone();
    let start_url = app.start_url.clone();

    let webview = WebViewBuilder::new_with_web_context(web_context)
        .with_url(&start_url)
        .with_new_window_req_handler(move |popup_url, _features| {
            match classify_popup(&popup_url, popup_parent_origin.as_deref()) {
                PopupDisposition::Managed => NewWindowResponse::Allow,
                PopupDisposition::SystemBrowser => {
                    if let Err(e) = open::that(&popup_url) {
                        tracing::warn!(?e, %popup_url, "failed to open system browser");
                    }
                    NewWindowResponse::Deny
                }
            }
        })
        .with_ipc_handler(move |req| {
            let page_host = req.uri().host();
            let origin_ok = match (&bridge_host, page_host) {
                (Some(expected), Some(host)) => {
                    host == expected || host.ends_with(&format!(".{expected}"))
                }
                _ => false,
            };
            if !origin_ok {
                return;
            }
            let response_json = bridge.handle(req.body());
            let script = format!(
                "window.dispatchEvent(new CustomEvent('reeve-response', {{ detail: {response_json} }}));"
            );
            let _ = bridge_proxy.send_event(UserEvent::BridgeResponse { window_id, script });
        })
        .build(&window)?;

    Ok((window, webview, instance, started_at))
}