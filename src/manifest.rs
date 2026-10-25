use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use scraper::{Html, Selector};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Deserialize)]
struct Manifest {
    name: Option<String>,
    short_name: Option<String>,
    icons: Option<Vec<ManifestIcon>>,
}

#[derive(Debug, Deserialize)]
struct ManifestIcon {
    src: String,
    sizes: Option<String>,
    #[serde(rename = "type")]
    mime_type: Option<String>,
}

pub struct ManifestData {
    pub name: Option<String>,
    pub icon_path: Option<PathBuf>,
}

pub fn fetch_manifest_data(
    start_url: &str,
    app_id: &str,
    data_dir: &Path,
) -> Result<ManifestData> {
    let client = Client::builder()
        .user_agent("Reeve/0.3 PWA Launcher")
        .build()?;

    let page_url = Url::parse(start_url)?;
    let resp = client.get(page_url.clone()).send()?;
    let html = resp.text()?;

    let document = Html::parse_document(&html);
    let selector = Selector::parse(r#"link[rel="manifest"]"#).unwrap();
    let manifest_href = document
        .select(&selector)
        .next()
        .and_then(|el| el.value().attr("href"))
        .ok_or_else(|| anyhow!("no <link rel=\"manifest\"> found"))?;

    let manifest_url = page_url.join(manifest_href)?;
    let manifest_resp = client.get(manifest_url.clone()).send()?;
    let manifest_text = manifest_resp.text()?;
    let manifest: Manifest = serde_json::from_str(&manifest_text)?;

    let name = manifest.name.or(manifest.short_name);

    let icon_path = if let Some(icons) = manifest.icons {
        let best = icons
            .iter()
            .filter(|i| {
                let s = i.src.to_lowercase();
                s.ends_with(".png") || s.ends_with(".ico")
            })
            .max_by_key(|i| parse_size(i.sizes.as_deref()))
            .or_else(|| icons.first());

        if let Some(icon) = best {
            let icon_url = manifest_url.join(&icon.src)?;
            let icon_resp = client.get(icon_url.clone()).send()?;
            let bytes = icon_resp.bytes()?;

            let icons_dir = data_dir.join("icons");
            fs::create_dir_all(&icons_dir)?;

            let ext = if let Some(mime) = &icon.mime_type {
                if mime.contains("png") {
                    "png"
                } else if mime.contains("ico") || mime.contains("x-icon") {
                    "ico"
                } else {
                    "img"
                }
            } else if icon_url.path().to_lowercase().ends_with(".ico") {
                "ico"
            } else {
                "png"
            };

            let path = icons_dir.join(format!("{}.{}", app_id, ext));
            fs::write(&path, &bytes)?;
            Some(path)
        } else {
            None
        }
    } else {
        None
    };

    Ok(ManifestData { name, icon_path })
}

fn parse_size(sizes: Option<&str>) -> u32 {
    sizes
        .and_then(|s| s.split('x').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}