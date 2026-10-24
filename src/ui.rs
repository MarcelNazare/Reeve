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
    OpenPopup {
        parent_window_id: WindowId,
        parent_app_id: String,
        parent_origin: Option<String>,
        url: String,
    },
    ClosePopup(WindowId),
}

struct AppWindow {
    window: Window,
    webview: WebView,
    instance: AppInstance,
}

struct PopupWindow {
    window: Window,
    _webview: WebView,
    parent_id: WindowId,
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
    let mut popup_windows: HashMap<WindowId, PopupWindow> = HashMap::new();
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

            Event::UserEvent(UserEvent::OpenPopup {
                parent_window_id,
                parent_app_id,
                parent_origin,
                url,
            }) => {
                match classify_popup(&url, parent_origin.as_deref()) {
                    PopupDisposition::Managed => {
                        tracing::info!(%url, "spawning managed popup");
                        match spawn_popup_window(
                            event_loop,
                            &mut web_context,
                            parent_window_id,
                            parent_app_id.clone(),
                            url.clone(),
                            proxy.clone(),
                        ) {
                            Ok((window, webview)) => {
                                let window_id = window.id();
                                popup_windows.insert(
                                    window_id,
                                    PopupWindow {
                                        window,
                                        _webview: webview,
                                        parent_id: parent_window_id,
                                    },
                                );
                            }
                            Err(e) => tracing::error!(?e, %url, "failed to spawn popup"),
                        }
                    }
                    PopupDisposition::SystemBrowser => {
                        tracing::info!(%url, "routing external popup to system browser");
                        if let Err(e) = open::that(&url) {
                            tracing::warn!(?e, %url, "failed to open system browser");
                        }
                    }
                }
            }

            Event::UserEvent(UserEvent::ClosePopup(window_id)) => {
                if popup_windows.remove(&window_id).is_some() {
                    tracing::info!("popup closed via JS");
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

                    // Reap popups that belonged to this app.
                    let orphans: Vec<WindowId> = popup_windows
                        .iter()
                        .filter(|(_, pw)| pw.parent_id == window_id)
                        .map(|(id, _)| *id)
                        .collect();
                    for id in orphans {
                        popup_windows.remove(&id);
                    }

                    tracing::info!(app_id = %aw.instance.app.id, "app closed");
                } else if popup_windows.remove(&window_id).is_some() {
                    tracing::info!("popup closed");
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
                        tracing::info!(?pids, "renderers attached");
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
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return PopupDisposition::SystemBrowser,
    };

    if parsed.scheme() == "about" {
        return PopupDisposition::Managed;
    }

    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return PopupDisposition::SystemBrowser;
    }

    let host = match parsed.host_str() {
        Some(h) => h.to_lowercase(),
        None => return PopupDisposition::SystemBrowser,
    };

    if let Some(expected) = parent_origin {
        let expected = expected.to_lowercase();
        if host == expected || host.ends_with(&format!(".{expected}")) {
            return PopupDisposition::Managed;
        }
    }

    const OAUTH_HOSTS: &[&str] = &[
        "accounts.google.com",
        "login.microsoftonline.com",
        "login.live.com",
        "appleid.apple.com",
        "auth0.com",
        "okta.com",
    ];
    for allowed in OAUTH_HOSTS {
        if host == *allowed || host.ends_with(&format!(".{allowed}")) {
            return PopupDisposition::Managed;
        }
    }

    PopupDisposition::SystemBrowser
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

    let popup_proxy = proxy.clone();
    let popup_app_id = app.id.clone();
    let popup_parent_origin = expected_host.clone();

    let start_url = app.start_url.clone();

    let webview = WebViewBuilder::new_with_web_context(web_context)
        .with_url(&start_url)
        .with_new_window_req_handler(move |popup_url, _features| {
            // IMPORTANT: returning Allow creates a dummy popup in WebView2 that
            // closes immediately. We must Deny and spawn a real window ourselves.
            let _ = popup_proxy.send_event(UserEvent::OpenPopup {
                parent_window_id: window_id,
                parent_app_id: popup_app_id.clone(),
                parent_origin: popup_parent_origin.clone(),
                url: popup_url,
            });
            NewWindowResponse::Deny
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

fn spawn_popup_window(
    event_loop: &EventLoopWindowTarget<UserEvent>,
    web_context: &mut WebContext,
    parent_window_id: WindowId,
    app_id: String,
    url: String,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<(Window, WebView)> {
    let window = WindowBuilder::new()
        .with_title(format!("{app_id} — Reeve"))
        .with_inner_size(tao::dpi::LogicalSize::new(820.0, 650.0))
        .build(event_loop)?;
    let window_id = window.id();

    // Popups do not get the capability bridge. They exist for auth flows and
    // transient UI; native capabilities belong to the parent app.
    let close_proxy = proxy.clone();

    let webview = WebViewBuilder::new_with_web_context(web_context)
        .with_url(&url)
        .with_initialization_script(
            r#"
            (function() {
                const nativeClose = window.close.bind(window);
                window.close = function() {
                    try {
                        window.ipc.postMessage(JSON.stringify({ cmd: 'reeve_popup_close' }));
                    } catch (e) {}
                    try { nativeClose(); } catch (e) {}
                };
            })();
            "#,
        )
        .with_ipc_handler(move |req| {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(req.body()) {
                if msg["cmd"] == "reeve_popup_close" {
                    let _ = close_proxy.send_event(UserEvent::ClosePopup(window_id));
                }
            }
        })
        .with_new_window_req_handler(|nested_url, _features| {
            tracing::warn!(%nested_url, "popup tried to open another popup — blocked");
            NewWindowResponse::Deny
        })
        .build(&window)?;

    let _ = parent_window_id; // stored by caller for cleanup

    Ok((window, webview))
}