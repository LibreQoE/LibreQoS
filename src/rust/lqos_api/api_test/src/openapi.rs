use anyhow::{Context, Result};

use crate::client::ApiClient;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Operation {
    pub method: String,
    pub path: String,
}

pub async fn fetch_operations(client: &ApiClient) -> Result<Vec<Operation>> {
    let resp = client
        .get("/openapi.json")
        .await
        .context("fetch /openapi.json")?;
    if !resp.status.is_success() {
        anyhow::bail!(
            "openapi fetch failed: {} {}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp.body).context("parse openapi.json")?;

    let paths = v
        .get("paths")
        .and_then(|p| p.as_object())
        .context("openapi missing 'paths' object")?;

    let mut ops = Vec::new();
    for (path, methods) in paths {
        let Some(methods) = methods.as_object() else {
            continue;
        };
        for (method, _op) in methods {
            let m = method.to_ascii_uppercase();
            if matches!(m.as_str(), "GET" | "POST" | "PUT" | "DELETE" | "PATCH") {
                ops.push(Operation {
                    method: m,
                    path: path.clone(),
                });
            }
        }
    }

    ops.sort_by(|a, b| (a.method.as_str(), a.path.as_str()).cmp(&(b.method.as_str(), b.path.as_str())));
    Ok(ops)
}

