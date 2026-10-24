use crate::bridge::Bridge;
use crate::catalog::{AppRecord, Catalog};
use crate::environment::SharedEnvironment;
use crate::governor::win::{enumerate_and_attach_renderers, Governor};
use crate::instance::{AppInstance, Lifecycle};
use anyhow::Result;
use std::collections::HashMap;
use std::rc::Rc;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder};

#[derive(Debug, Clone)]
pub enum UserEvent {
    Launch(String),
    BridgeResponse { window_id: WindowId, script: String },
}

struct AppWindow {
    window: Window,
    webview: WebView,
    instance: AppInstance,
}

const LAUNCHER_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  * { margin:0; padding:0; box-sizing:border-box; }
  body { font-family: 'Segoe UI', system-ui, sans-serif; background:#0f0f11; color:#e8e8ec; padding:24px; }
  h1 { font-size:18px; font-weight:600; margin-bottom:4px; color:#fff; }
  .sub { font-size:12px; color:#6b6b76; margin-bottom:20px; }
  .grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(140px,1fr)); gap:12px; }
  .app { background:#1a1a1f; border:1px solid #2a2a32; border-radius:10px; padding:16px; cursor:pointer; transition:all .15s; }
  .app:hover { border-color:#4a4a5a; background:#202028; }
  .app .name { font-size:13px; font-weight:500; }
  .app .url { font-size:10px; color:#5a5a66; margin-top:4px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .empty { color:#5a5a66; font-size:13px; padding:40px 0; text-align:center; }
</style>
</head>
<body>
  <h1>Reeve</h1>
  <p class="sub">One engine. Many apps. Governed.</p>
  <div class="grid" id="grid"></div>
  <p class="empty" id="empty" style="display:none">No apps installed yet.</p>
<script>
  function launch(id) { window.ipc.postMessage(JSON.stringify({cmd:'launch', app_id:id})); }
  function render(apps) {
    var grid = document.getElementById('grid');
    var empty = document.getElementById('empty');
    if (!apps.length) { empty.style.display='block'; return; }
    empty.style.display='none';
    grid.innerHTML = apps.map(function(a) {
      return '<div class="app" onclick="launch(\''+a.id+'\')">' +
        '<div class="name">'+a.name+'</div><div class="url">'+a.start_url+'</div></div>';
    }).join('');
  }
  window.__reeve_render = render;
</script>
</body>
</html>"#;

pub fn run_launcher(catalog: Catalog, env: SharedEnvironment, governor: Governor) -> Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let apps = catalog.list_apps()?;
    let apps_json = serde_json::to_string(&apps)?;

    let launcher_window = WindowBuilder::new()
        .with_title("Reeve")
        .with_inner_size(tao::dpi::LogicalSize::new(720.0, 520.0))
        .build(&event_loop)?;

    let mut web_context = WebContext::new(Some(env.data_dir.clone()));

    let launcher_webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_html(LAUNCHER_HTML)
        .with_ipc_handler({
            let proxy = proxy.clone();
            move |req| {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(req.body()) {
                    if msg["cmd"] == "launch" {
                        if let Some(app_id) = msg["app_id"].as_str() {
                            let _ = proxy.send_event(UserEvent::Launch(app_id.to_string()));
                        }
                    }
                }
            }
        })
        .build(&launcher_window)?;

    launcher_webview.evaluate_script(&format!("window.__reeve_render({apps_json})"))?;

    let _launcher_keepalive = (launcher_window, launcher_webview);

    let mut app_windows: HashMap<WindowId, AppWindow> = HashMap::new();
    let governor = Rc::new(governor);
    let catalog = Rc::new(catalog);

    let mut last_scan = std::time::Instant::now();
    let scan_interval = std::time::Duration::from_secs(3);

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(500),
        );

        match event {
            Event::UserEvent(UserEvent::Launch(app_id)) => {
                if let Some(aw) = app_windows.values().find(|aw| aw.instance.app.id == app_id) {
                    let _ = aw.window.set_focus();
                    tracing::info!(%app_id, "app already open — focusing existing window");
                    return;
                }

                let app = match catalog
                    .list_apps()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|a| a.id == app_id)
                {
                    Some(a) => a,
                    None => {
                        tracing::warn!(%app_id, "unknown app id");
                        return;
                    }
                };

                match spawn_app_window(
                    event_loop,
                    &mut web_context,
                    app.clone(),
                    proxy.clone(),
                    catalog.clone(),
                ) {
                    Ok((window, webview, mut instance)) => {
                        instance.transition(Lifecycle::Ready);
                        tracing::info!(app_id = %app.id, session = instance.session_id, "app spawned");
                        app_windows.insert(
                            window.id(),
                            AppWindow { window, webview, instance },
                        );
                    }
                    Err(e) => tracing::error!(?e, %app_id, "failed to spawn app"),
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
                    tracing::debug!(app_id = %aw.instance.app.id, focused, "app focus changed");
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
            last_scan = std::time::Instant::now();
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
    });
}

// --- popup classification ---

enum PopupDisposition {
    Managed,
    SystemBrowser,
}

fn classify_popup(url: &str, parent_origin: Option<&str>) -> PopupDisposition {
    if url.is_empty() {
        tracing::info!("popup URL is empty — Managed");
        return PopupDisposition::Managed;
    }

    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(raw = %url, error = %e, "popup URL parse failed — SystemBrowser");
            return PopupDisposition::SystemBrowser;
        }
    };

    let scheme = parsed.scheme();

    // about:, blob:, data: — JS-driven windows.
    if scheme == "about" || scheme == "blob" || scheme == "data" {
        tracing::info!(%url, scheme, "opaque-scheme popup — Managed");
        return PopupDisposition::Managed;
    }

    if scheme != "https" && scheme != "http" {
        tracing::info!(%url, scheme, "non-http popup — SystemBrowser");
        return PopupDisposition::SystemBrowser;
    }

    let host = match parsed.host_str() {
        Some(h) => h.trim_end_matches('.').to_lowercase(),
        None => {
            tracing::info!(%url, "popup has no host — SystemBrowser");
            return PopupDisposition::SystemBrowser;
        }
    };

    // Gate 1: exact-origin (host == parent, or host ends with ".parent").
    if let Some(expected) = parent_origin {
        let expected = expected.trim_end_matches('.').to_lowercase();
        if !expected.is_empty() && (host == expected || host.ends_with(&format!(".{expected}"))) {
            tracing::info!(%url, %host, %expected, "same-origin popup — Managed");
            return PopupDisposition::Managed;
        }

        // Gate 2: same-brand — second-level label matches. Catches
        // notion.so <-> app.notion.com and similar TLD migrations.
        if same_brand_domain(&host, &expected) {
            tracing::info!(%url, %host, %expected, "same-brand popup — Managed");
            return PopupDisposition::Managed;
        }
    }

    // Gate 3: known OAuth / identity-provider hosts.
    const OAUTH_HOSTS: &[&str] = &[
        "accounts.google.com",
        "ogs.google.com",
        "apis.google.com",
        "login.microsoftonline.com",
        "login.live.com",
        "login.windows.net",
        "microsoftonline.com",
        "appleid.apple.com",
        "idmsa.apple.com",
        "auth0.com",
        "okta.com",
        "github.com",
        "githubusercontent.com",
        "slack.com",
        "slack-edge.com",
        "facebook.com",
        "twitter.com",
        "x.com",
        "linkedin.com",
        "discord.com",
        "notion.so",
        "notion.com",
        "linear.app",
        "figma.com",
    ];
    for allowed in OAUTH_HOSTS {
        if host == *allowed || host.ends_with(&format!(".{allowed}")) {
            tracing::info!(%url, %host, matched = %allowed, "oauth-host popup — Managed");
            return PopupDisposition::Managed;
        }
    }

    // Gate 4: OAuth / popup-flow query heuristic (case-insensitive).
    if looks_like_oauth(&parsed) {
        tracing::info!(%url, %host, "oauth-query popup — Managed");
        return PopupDisposition::Managed;
    }

    tracing::info!(%url, %host, "external popup — SystemBrowser");
    PopupDisposition::SystemBrowser
}

/// Compare the second-level label of two hosts, e.g.
/// "app.notion.com" and "notion.so" both yield "notion".
/// Handles "localhost" (no second-level label) by returning false.
fn same_brand_domain(host: &str, parent: &str) -> bool {
    let host_label = host.split('.').rev().nth(1).unwrap_or("").to_lowercase();
    let parent_label = parent.split('.').rev().nth(1).unwrap_or("").to_lowercase();
    !host_label.is_empty()
        && !parent_label.is_empty()
        && host_label != "co"
        && host_label != "com"
        && host_label == parent_label
}

/// Detects OAuth / popup-flow URLs by their query parameters, case-insensitively.
/// Handles both snake_case (redirect_uri) and camelCase (redirectUri) forms,
/// which Notion and several other providers use.
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
    let has_response_type =
        query.contains("response_type=") || query.contains("responsetype=");
    let has_scope = query.contains("scope=");
    let has_state = query.contains("state=");
    let has_popup_marker = query.contains("callbacktype=popup")
        || query.contains("popupflowid=")
        || query.contains("popup=true")
        || query.contains("popup_type=");

    if has_popup_marker {
        return true;
    }
    if has_client_id && has_redirect {
        return true;
    }
    if has_response_type && has_scope {
        return true;
    }
    if has_state && (has_client_id || has_response_type) {
        return true;
    }
    // Any redirect param on its own is a strong indicator of a popup/redirect
    // flow — legitimate external links rarely carry redirect=... markers.
    if has_redirect {
        return true;
    }
    false
}

// --- window construction ---

fn spawn_app_window(
    event_loop: &EventLoopWindowTarget<UserEvent>,
    web_context: &mut WebContext,
    app: AppRecord,
    proxy: EventLoopProxy<UserEvent>,
    catalog: Rc<Catalog>,
) -> Result<(Window, WebView, AppInstance)> {
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

    let bridge_host = expected_host.clone();
    let bridge_proxy = proxy.clone();

    let popup_parent_origin = expected_host.clone();
    let start_url = app.start_url.clone();

    let webview = WebViewBuilder::new_with_web_context(web_context)
        .with_url(&start_url)
        .with_new_window_req_handler(move |popup_url, _features| {
            tracing::info!(raw = %popup_url, "NewWindowRequested");

            match classify_popup(&popup_url, popup_parent_origin.as_deref()) {
                PopupDisposition::Managed => {
                    tracing::info!(%popup_url, "allowing WebView2 to open popup natively");
                    NewWindowResponse::Allow
                }
                PopupDisposition::SystemBrowser => {
                    tracing::info!(%popup_url, "routing external popup to system browser");
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
                tracing::debug!(?page_host, ?bridge_host, "IPC rejected: origin mismatch");
                return;
            }

            let response_json = bridge.handle(req.body());
            let script = format!(
                "window.dispatchEvent(new CustomEvent('reeve-response', {{ detail: {response_json} }}));"
            );

            let _ = bridge_proxy.send_event(UserEvent::BridgeResponse { window_id, script });
        })
        .build(&window)?;

    Ok((window, webview, instance))
}