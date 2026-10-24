use anyhow::Result;
use std::path::PathBuf;

/// One shared WebView environment. Every app's WebView must use the same
/// data directory, or WebView2 will spawn a separate browser process tree
/// per app, defeating the entire memory-sharing story.
pub struct SharedEnvironment {
    pub data_dir: PathBuf,
}

impl SharedEnvironment {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        Ok(Self { data_dir })
    }
}