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

    // Always re-seed. add_app is INSERT OR REPLACE keyed on id, so this is
    // idempotent: missing apps are added, existing seeds have their
    // name/start_url refreshed, and custom apps added via the search bar
    // (different ids) are left untouched.
    seed_catalog(&catalog)?;

    let governor = governor::win::Governor::new(1_073_741_824, 4_294_967_296)?;

    tracing::info!(data = %data.display(), "Reeve starting");
    ui::run_launcher(catalog, env, governor)
}

fn seed_catalog(catalog: &catalog::Catalog) -> Result<()> {
    // --- productivity ---
    catalog.add_app("linear", "Linear", "https://linear.app")?;
    catalog.add_app("notion", "Notion", "https://notion.so")?;
    catalog.add_app("figma", "Figma", "https://figma.com")?;
    catalog.add_app("miro", "Miro", "https://miro.com/app")?;
    catalog.add_app("excalidraw", "Excalidraw", "https://excalidraw.com")?;
    catalog.add_app("trello", "Trello", "https://trello.com")?;
    catalog.add_app("asana", "Asana", "https://app.asana.com")?;
    catalog.add_app("clickup", "ClickUp", "https://app.clickup.com")?;
    catalog.add_app("todoist", "Todoist", "https://app.todoist.com")?;

    // --- communication ---
    catalog.add_app("slack", "Slack", "https://app.slack.com/client")?;
    catalog.add_app("discord", "Discord", "https://discord.com/app")?;
    catalog.add_app("teams", "Microsoft Teams", "https://teams.microsoft.com")?;
    catalog.add_app("whatsapp", "WhatsApp", "https://web.whatsapp.com")?;
    catalog.add_app("telegram", "Telegram", "https://web.telegram.org")?;
    catalog.add_app("zoom", "Zoom", "https://app.zoom.us")?;

    // --- media ---
    catalog.add_app("spotify", "Spotify", "https://open.spotify.com")?;
    catalog.add_app("ytmusic", "YouTube Music", "https://music.youtube.com")?;
    catalog.add_app("soundcloud", "SoundCloud", "https://soundcloud.com")?;
    catalog.add_app("twitch", "Twitch", "https://www.twitch.tv")?;

    // --- google workspace ---
    catalog.add_app("gmail", "Gmail", "https://mail.google.com")?;
    catalog.add_app("gcal", "Calendar", "https://calendar.google.com")?;
    catalog.add_app("gdrive", "Drive", "https://drive.google.com")?;
    catalog.add_app("gdocs", "Docs", "https://docs.google.com")?;
    catalog.add_app("gkeep", "Keep", "https://keep.google.com")?;

    // --- dev tools ---
    catalog.add_app("vscode", "VS Code", "https://vscode.dev")?;
    catalog.add_app("github", "GitHub", "https://github.com")?;
    catalog.add_app("codesandbox", "CodeSandbox", "https://codesandbox.io")?;
    catalog.add_app("stackblitz", "StackBlitz", "https://stackblitz.com")?;
    catalog.add_app("photopea", "Photopea", "https://www.photopea.com")?;

    // --- social ---
    catalog.add_app("bsky", "Bluesky", "https://bsky.app")?;
    catalog.add_app("reddit", "Reddit", "https://www.reddit.com")?;
    catalog.add_app("mastodon", "Mastodon", "https://mastodon.social")?;

    // --- permissions ---
    catalog.grant_permission("linear", "notify", "")?;
    catalog.grant_permission("figma", "notify", "")?;
    catalog.grant_permission("slack", "notify", "")?;
    catalog.grant_permission("discord", "notify", "")?;
    catalog.grant_permission("teams", "notify", "")?;

    catalog.grant_permission("notion", "fs.pick", "")?;
    catalog.grant_permission("notion", "fs.read", r"C:\Users\Public")?;
    catalog.grant_permission("notion", "fs.write", r"C:\Users\Public")?;

    catalog.grant_permission("excalidraw", "fs.pick", "")?;
    catalog.grant_permission("excalidraw", "fs.write", r"C:\Users\Public")?;

    catalog.grant_permission("photopea", "fs.pick", "")?;
    catalog.grant_permission("photopea", "fs.write", r"C:\Users\Public")?;

    Ok(())
}