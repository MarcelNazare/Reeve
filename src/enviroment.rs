use anyhow::Result;
use std::path::PathBuf;

pub struct SharedEnvironment {
    pub data_dir: PathBuf,
}

impl SharedEnvironment {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        Ok(Self { data_dir })
    }
}