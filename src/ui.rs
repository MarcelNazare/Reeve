use crate::bridge::Bridge;
use crate::catalog::{AppRecord, Catalog};
use crate::environment::SharedEnvironment;
use crate::governor::win::{enumerate_and_attach_renderers, Governor};
use crate::instance::{AppInstance, Lifecycle};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{WebContext, WebView, WebViewBuilder};

#[derive(Debug, Clone)]
pub enum UserEvent {
    Launch(String),
    BridgeResponse { window_id: WindowId, script: String },
}

struct AppWindow {
    window: Window,
    webview: WebView,
    instance: AppInstance,
    _expected_host: Option<String>,
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

    // One WebContext for the entire process. Every WebView — launcher and
    // every app — must share this context to share the browser process.
    let mut web_context = WebContext::new(Some(env.data_dir.clone()));

    let launcher_webview = WebViewBuilder::new()
        .with_web_context(&mut web_context)
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

    // Keep launcher resources alive for the entire event loop.
    let _launcher_keepalive = (launcher_window, launcher_webview);

    let mut app_windows: HashMap<WindowId, AppWindow> = HashMap::new();
    let governor = Arc::new(governor);
    let catalog = Arc::new(catalog);

    let mut last_scan = std::time::Instant::now();
    let scan_interval = std::time::Duration::from_secs(3);

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(500),
        );

        match event {
            Event::UserEvent(UserEvent::Launch(app_id)) => {
                if app_windows
                    .values()
                    .any(|aw| aw.instance.app.id == app_id)
                {
                    tracing::info!(%app_id, "app already open");
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
                    Ok((window, webview, instance, expected_host)) => {
                        tracing::info!(app_id = %app.id, session = instance.session_id, "app spawned");
                        app_windows.insert(
                            window.id(),
                            AppWindow {
                                window,
                                webview,
                                instance,
                                _expected_host: expected_host,
                            },
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
                    // Launcher close => exit.
                    *control_flow = ControlFlow::Exit;
                }
            }

            Event::LoopDestroyed => {
                tracing::info!("event loop destroyed");
            }

            _ => {}
        }

        // Periodic renderer attachment. New renderers spawn as pages grow.
        if last_scan.elapsed() >= scan_interval {
            last_scan = std::time::Instant::now();
            for (_, aw) in &app_windows {
                match enumerate_and_attach_renderers(&governor, &aw.webview) {
                    Ok(pids) if !pids.is_empty() => {
                        tracing::info!(
                            app_id = %aw.instance.app.id,
                            ?pids,
                            "renderers attached"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(
                            ?e,
                            app_id = %aw.instance.app.id,
                            "renderer enumeration failed"
                        );
                    }
                }
            }
        }
    });
}

fn spawn_app_window(
    event_loop: &EventLoopWindowTarget<UserEvent>,
    web_context: &mut WebContext,
    app: AppRecord,
    proxy: EventLoopProxy<UserEvent>,
    catalog: Arc<Catalog>,
) -> Result<(Window, WebView, AppInstance, Option<String>)> {
    let window = WindowBuilder::new()
        .with_title(app.name.clone())
        .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 750.0))
        .build(event_loop)?;
    let window_id = window.id();

    // Preload the permission map so the bridge is `Send + 'static`.
    let mut permissions: HashMap<String, Vec<String>> = HashMap::new();
    for (cap, scope) in catalog.get_all_permissions(&app.id).unwrap_or_default() {
        permissions.entry(cap).or_default().push(scope);
    }
    let bridge = Bridge::new(app.id.clone(), permissions);

    // Expected origin for IPC filtering. Blocks third-party pages from
    // invoking the bridge during OAuth redirects or malicious navigations.
    let expected_host = url::Url::parse(&app.start_url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()));

    // Register the session before spawn so its id is available on close.
    let session_id = catalog.record_session_start(&app.id, 0).unwrap_or(-1);
    let instance = AppInstance::spawn(app.clone(), session_id)?;

    let handler_host = expected_host.clone();
    let handler_proxy = proxy.clone();
    let start_url = app.start_url.clone();

    let webview = WebViewBuilder::new()
        .with_web_context(web_context)
        .with_url(&start_url)
        .with_ipc_handler(move |req| {
            // Origin gate — reject IPC from unexpected hosts.
            let page_host = req.uri().host();
            let origin_ok = match (&handler_host, page_host) {
                (Some(expected), Some(host)) => {
                    host == expected || host.ends_with(&format!(".{expected}"))
                }
                _ => false,
            };
            if !origin_ok {
                tracing::debug!(
                    ?page_host,
                    ?handler_host,
                    "IPC rejected: origin mismatch"
                );
                return;
            }

            let response_json = bridge.handle(req.body());
            let script = format!(
                "window.dispatchEvent(new CustomEvent('reeve-response', {{ detail: {response_json} }}));"
            );

            let _ = handler_proxy.send_event(UserEvent::BridgeResponse {
                window_id,
                script,
            });
        })
        .build(&window)?;

    Ok((window, webview, instance, expected_host))
}