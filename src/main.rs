mod bridge;
mod catalog;
mod environment;
mod governor;
mod instance;
mod ui;

use anyhow::Result;
use directories::ProjectDirs;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,reeve=debug".into()),
        )
        .init();

    let proj = ProjectDirs::from("dev", "Reeve", "Reeve")
        .ok_or_else(|| anyhow::anyhow!("no project directories available"))?;
    let data = proj.data_dir().to_path_buf();
    std::fs::create_dir_all(&data)?;

    let catalog = catalog::Catalog::open(&data.join("catalog.db"))?;
    let env = environment::SharedEnvironment::new(data.join("webview"))?;

    if catalog.list_apps()?.is_empty() {
        catalog.add_app("linear", "Linear", "https://linear.app")?;
        catalog.add_app("notion", "Notion", "https://notion.so")?;
        catalog.add_app("figma", "Figma", "https://figma.com")?;

        catalog.grant_permission("linear", "notify", "")?;
        catalog.grant_permission("notion", "fs.pick", "")?;
        catalog.grant_permission("notion", "fs.read", r"C:\Users\Public")?;
        catalog.grant_permission("notion", "fs.write", r"C:\Users\Public")?;
        catalog.grant_permission("figma", "notify", "")?;
    }

    let governor = governor::win::Governor::new(1_073_741_824, 4_294_967_296)?;

    tracing::info!(data = %data.display(), "Reeve starting");
    ui::run_launcher(catalog, env, governor)
}