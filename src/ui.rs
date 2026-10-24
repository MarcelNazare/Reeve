use crate::{catalog::Catalog, environment::SharedEnvironment};
use anyhow::Result;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

pub fn run_launcher(catalog: Catalog, env: SharedEnvironment) -> Result<()> {
    let event_loop = EventLoop::new();
    let apps = catalog.list_apps()?;

    // Milestone 1: one window, one controller, proving the shared env works.
    // Milestone 2: one window per app, all with the same data_directory.
    let first = apps.first().expect("seeded catalog");
    let window = WindowBuilder::new()
        .with_title(format!("Reeve — {}", first.name))
        .build(&event_loop)?;

    let _webview = WebViewBuilder::new()
        .with_url(&first.start_url)
        .with_data_directory(&env.data_dir)
        .build(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *control_flow = ControlFlow::Exit;
        }
    });
}