use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct BridgeRequest {
    pub capability: String,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BridgeResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

pub struct Bridge {
    app_id: String,
    permissions: HashMap<String, Vec<String>>,
}

impl Bridge {
    pub fn new(app_id: String, permissions: HashMap<String, Vec<String>>) -> Self {
        Self { app_id, permissions }
    }

    pub fn handle(&self, body: &str) -> String {
        let req: BridgeRequest = match serde_json::from_str(body) {
            Ok(r) => r,
            Err(e) => {
                return serde_json::to_string(&BridgeResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("invalid request: {e}")),
                    request_id: None,
                })
                .unwrap_or_else(|_| r#"{"ok":false,"error":"serialization failure"}"#.into());
            }
        };

        let request_id = req.request_id.clone();
        let response = match self.dispatch(&req) {
            Ok(data) => BridgeResponse {
                ok: true,
                data: Some(data),
                error: None,
                request_id,
            },
            Err(e) => BridgeResponse {
                ok: false,
                data: None,
                error: Some(e.to_string()),
                request_id,
            },
        };

        serde_json::to_string(&response)
            .unwrap_or_else(|_| r#"{"ok":false,"error":"serialization failure"}"#.into())
    }

    fn dispatch(&self, req: &BridgeRequest) -> Result<serde_json::Value> {
        match req.capability.as_str() {
            "fs.pick" => self.fs_pick(),
            "fs.read" => self.fs_read(&req.args),
            "fs.write" => self.fs_write(&req.args),
            "notify" => self.notify(&req.args),
            other => Err(anyhow!("unknown capability: {other}")),
        }
    }

    fn fs_pick(&self) -> Result<serde_json::Value> {
        self.require_capability("fs.pick", None)?;
        match rfd::FileDialog::new().pick_files() {
            Some(paths) => Ok(serde_json::json!({
                "paths": paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>()
            })),
            None => Ok(serde_json::json!({ "paths": [] })),
        }
    }

    fn fs_read(&self, args: &serde_json::Value) -> Result<serde_json::Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow!("fs.read requires a 'path' argument"))?;
        self.require_capability("fs.read", Some(path))?;
        let content = std::fs::read_to_string(path)?;
        Ok(serde_json::json!({ "content": content }))
    }

    fn fs_write(&self, args: &serde_json::Value) -> Result<serde_json::Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow!("fs.write requires a 'path' argument"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| anyhow!("fs.write requires a 'content' argument"))?;
        self.require_capability("fs.write", Some(path))?;
        std::fs::write(path, content)?;
        Ok(serde_json::json!({ "written": path }))
    }

    fn notify(&self, args: &serde_json::Value) -> Result<serde_json::Value> {
        self.require_capability("notify", None)?;
        let title = args["title"].as_str().unwrap_or("Reeve");
        let body = args["body"].as_str().unwrap_or("");
        notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .show()?;
        Ok(serde_json::json!({ "shown": true }))
    }

    fn require_capability(&self, capability: &str, path: Option<&str>) -> Result<()> {
        let scopes = self.permissions.get(capability).ok_or_else(|| {
            anyhow!(
                "capability '{capability}' is not granted to '{}'",
                self.app_id
            )
        })?;

        if let Some(p) = path {
            let requested = std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
            let allowed = scopes.iter().any(|scope| {
                if scope.is_empty() {
                    return false;
                }
                let scope_path =
                    std::fs::canonicalize(scope).unwrap_or_else(|_| PathBuf::from(scope));
                requested.starts_with(&scope_path)
            });
            if !allowed {
                return Err(anyhow!(
                    "path '{}' is outside the granted scope for '{}'",
                    p,
                    capability
                ));
            }
        }

        Ok(())
    }
}