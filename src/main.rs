mod catalog;
mod environment;
mod governor;
mod instance;
mod ui;

use anyhow::Result;
use directories::ProjectDirs;

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let proj = ProjectDirs::from("dev", "Reeve", "Reeve")
        .ok_or_else(|| anyhow::anyhow!("no project dirs"))?;
    let data = proj.data_dir().to_path_buf();
    std::fs::create_dir_all(&data)?;

    let catalog = catalog::Catalog::open(&data.join("catalog.db"))?;
    let env = environment::SharedEnvironment::new(data.join("webview"))?;

    // Seed one app so there's something to launch.
    if catalog.list_apps()?.is_empty() {
        catalog.add_app("linear", "Linear", "https://linear.app")?;
        catalog.add_app("notion", "Notion", "https://notion.so")?;
    }

    tracing::info!(apps = ?catalog.list_apps()?.iter().map(|a| &a.name).collect::<Vec<_>>(), "catalog ready");
    tracing::info!(env = %env.data_dir.display(), "shared environment ready");

    ui::run_launcher(catalog, env)
}