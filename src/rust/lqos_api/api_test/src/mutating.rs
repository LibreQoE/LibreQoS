use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use reqwest::StatusCode;
use sha2::Digest;

use crate::{client::ApiClient, openapi};

const SKIP_ALWAYS: &[(&str, &str)] = &[
    // These endpoints modify lqos.conf; even on a test node they are easy to misuse.
    ("POST", "/config/update_json"),
    ("POST", "/config/update_toml"),
];

struct FileBackup {
    path: PathBuf,
    original: Option<Vec<u8>>,
}

struct BackupGuard {
    files: Vec<FileBackup>,
    restored: bool,
}

impl BackupGuard {
    fn new(paths: Vec<PathBuf>) -> Result<Self> {
        let mut files = Vec::new();
        for path in paths {
            let original = match std::fs::read(&path) {
                Ok(b) => Some(b),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
            };
            files.push(FileBackup { path, original });
        }
        Ok(Self {
            files,
            restored: false,
        })
    }

    fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        for f in &self.files {
            match &f.original {
                Some(bytes) => {
                    std::fs::write(&f.path, bytes)
                        .with_context(|| format!("restore {}", f.path.display()))?;
                }
                None => {
                    if f.path.exists() {
                        std::fs::remove_file(&f.path)
                            .with_context(|| format!("remove {}", f.path.display()))?;
                    }
                }
            }
        }
        self.restored = true;
        Ok(())
    }

    async fn restore_and_reload(&mut self, client: &ApiClient) -> Result<()> {
        self.restore()?;
        let _ = client.post_empty("/reload_libreqos").await;
        Ok(())
    }
}

impl Drop for BackupGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn hash_file_or_empty(path: &Path) -> Result<String> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    let mut h = sha2::Sha256::new();
    h.update(&bytes);
    Ok(format!("{:x}", h.finalize()))
}

async fn wait_until_ready(client: &ApiClient) -> Result<()> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(60);
    loop {
        if start.elapsed() > timeout {
            anyhow::bail!("Timed out waiting for API readiness via GET /health");
        }
        match client.get("/health").await {
            Ok(resp) => match resp.status {
                StatusCode::OK => return Ok(()),
                StatusCode::FORBIDDEN => anyhow::bail!(
                    "API policy denied access (403): {}",
                    String::from_utf8_lossy(&resp.body)
                ),
                StatusCode::UNAUTHORIZED => anyhow::bail!(
                    "API returned 401 Unauthorized. Check the x-bearer credential in LQOS_API_KEY."
                ),
                _ => {}
            },
            Err(_) => {}
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

fn overrides_path_from_config() -> Result<PathBuf> {
    let cfg = lqos_config::load_config().context("load lqos.conf")?;
    Ok(Path::new(&cfg.lqos_directory).join("lqos_overrides.json"))
}

fn pick_operator_parent(network_json: &serde_json::Value) -> Option<String> {
    let ambiguous: HashSet<&str> = network_json
        .get("ambiguous_base_names")
        .and_then(|value| value.as_array())
        .map(|names| names.iter().filter_map(|name| name.as_str()).collect())
        .unwrap_or_default();
    network_json
        .get("nodes")?
        .as_array()?
        .iter()
        .find(|node| {
            node.get("type").and_then(|value| value.as_str()) != Some("circuit")
                && node
                    .get("name")
                    .and_then(|value| value.as_str())
                    .is_some_and(|name| !ambiguous.contains(name))
        })
        .and_then(|node| node.get("name").and_then(|value| value.as_str()))
        .map(str::to_string)
}

fn op(method: &str, path: &str) -> openapi::Operation {
    openapi::Operation {
        method: method.to_string(),
        path: path.to_string(),
    }
}

pub async fn run_mutating(client: &ApiClient, yes: bool) -> Result<()> {
    if !yes {
        anyhow::bail!(
            "Refusing to run mutating tests without --yes. These tests modify network.json/ShapedDevices.csv/overrides (then restore)."
        );
    }

    wait_until_ready(client).await?;

    // Resolve file paths for backup/restore.
    let network_json_path =
        lqos_config::NetworkJson::path().context("resolve network.json path")?;
    let shaped_devices_path =
        lqos_config::ConfigShapedDevices::path().context("resolve ShapedDevices.csv path")?;
    let overrides_path = overrides_path_from_config()?;

    let mut guard = BackupGuard::new(vec![
        network_json_path.clone(),
        shaped_devices_path.clone(),
        overrides_path.clone(),
    ])?;

    let result = run_mutating_inner(
        client,
        &network_json_path,
        &shaped_devices_path,
        &overrides_path,
    )
    .await;

    // Always restore, even if tests failed.
    guard.restore_and_reload(client).await?;

    result
}

async fn run_mutating_inner(
    client: &ApiClient,
    network_json_path: &Path,
    shaped_devices_path: &Path,
    overrides_path: &Path,
) -> Result<()> {
    let ops = openapi::fetch_operations(client).await?;

    let mut executed: HashSet<openapi::Operation> = HashSet::new();

    // Verify "live" endpoints don't touch overrides.
    let overrides_hash_before_live = hash_file_or_empty(overrides_path)?;

    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let circuit_id = format!("api_test_circuit_{suffix}");
    let device_id = format!("api_test_device_{suffix}");

    // Discover a real non-circuit parent from the published operator view.
    let network_raw = std::fs::read_to_string(network_json_path)
        .with_context(|| format!("read {}", network_json_path.display()))?;
    let network_json: serde_json::Value =
        serde_json::from_str(&network_raw).with_context(|| "parse operator view")?;
    let parent_node = pick_operator_parent(&network_json)
        .context("no non-circuit operator-view node found for the live circuit test")?;

    // --- Live shaped devices: upsert circuit (default reload=true) ---
    executed.insert(op("POST", "/shaped_devices/circuit"));
    let body = serde_json::json!({
        "circuit_id": circuit_id,
        "circuit_name": format!("api_test circuit {suffix}"),
        "device_id": device_id,
        "device_name": format!("api_test device {suffix}"),
        "parent_node": parent_node,
        "mac": "",
        "ipv4_list": ["100.64.0.250"],
        "ipv6_list": [],
        "download_min_mbps": 1.0,
        "upload_min_mbps": 1.0,
        "download_max_mbps": 10.0,
        "upload_max_mbps": 5.0,
        "comment": "api_test",
        "sqm_override": null
    });
    let resp = client.post_json("/shaped_devices/circuit", body).await?;
    if resp.status != StatusCode::OK {
        anyhow::bail!(
            "POST /shaped_devices/circuit failed: {} {}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
        );
    }
    let parsed: lqos_api::LiveUpdateResponse = serde_json::from_slice(&resp.body)?;
    anyhow::ensure!(
        parsed.reload_requested,
        "expected reload_requested=true by default for live upsert circuit"
    );

    // --- Live shaped devices: remove circuit (reload=false) ---
    executed.insert(op("DELETE", "/shaped_devices/circuit/{circuit_id}"));
    let resp = client
        .delete(&format!(
            "/shaped_devices/circuit/{}?reload=false",
            urlencoding::encode(&circuit_id)
        ))
        .await?;
    if resp.status != StatusCode::OK {
        anyhow::bail!(
            "DELETE /shaped_devices/circuit/{{circuit_id}} failed: {} {}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
        );
    }
    let parsed: lqos_api::LiveRemovedCountResponse = serde_json::from_slice(&resp.body)?;
    anyhow::ensure!(
        !parsed.reload_requested,
        "expected reload_requested=false when reload=false is provided"
    );

    let overrides_hash_after_live = hash_file_or_empty(overrides_path)?;
    anyhow::ensure!(
        overrides_hash_before_live == overrides_hash_after_live,
        "Overrides file changed during live (non-persistent) operations; expected no change"
    );

    // --- ShapedDevices.csv update (post existing file back) ---
    executed.insert(op("POST", "/shaped_devices/update"));
    let shaped_raw = std::fs::read_to_string(shaped_devices_path)
        .with_context(|| format!("read {}", shaped_devices_path.display()))?;
    let resp = client
        .post_text("/shaped_devices/update", shaped_raw)
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST /shaped_devices/update failed: {}",
        resp.status
    );

    // --- node_names ---
    executed.insert(op("POST", "/node_names"));
    let resp = client
        .post_json("/node_names", serde_json::json!([0]))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST /node_names failed: {}",
        resp.status
    );

    // --- Retired topology writes: ownership 409 stubs ---
    for (method, template, call_path) in [
        (
            "POST",
            "/network_json/update",
            "/network_json/update".to_string(),
        ),
        (
            "POST",
            "/network_json/node",
            "/network_json/node".to_string(),
        ),
        (
            "DELETE",
            "/network_json/node/{node_name}",
            format!(
                "/network_json/node/{}",
                urlencoding::encode("api_test_node")
            ),
        ),
        (
            "POST",
            "/network/reparent_site",
            "/network/reparent_site".to_string(),
        ),
    ] {
        executed.insert(op(method, template));
        let resp = if method == "DELETE" {
            client.delete(&call_path).await?
        } else {
            client.post_json(&call_path, serde_json::json!({})).await?
        };
        anyhow::ensure!(
            resp.status == StatusCode::CONFLICT,
            "{} {} expected 409, got {} {}",
            method,
            call_path,
            resp.status,
            String::from_utf8_lossy(&resp.body)
        );
    }

    // --- Control & reload ---
    executed.insert(op("POST", "/reload_libreqos"));
    let resp = client.post_empty("/reload_libreqos").await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST /reload_libreqos failed: {}",
        resp.status
    );

    // --- Urgent clears ---
    executed.insert(op("POST", "/urgent/clear_all"));
    let resp = client.post_empty("/urgent/clear_all").await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST /urgent/clear_all failed: {}",
        resp.status
    );

    executed.insert(op("DELETE", "/urgent/{id}"));
    let resp = client.delete("/urgent/0").await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE /urgent/{{id}} failed: {}",
        resp.status
    );

    // --- Overrides: persistent devices add/remove ---
    executed.insert(op("POST", "/overrides/persistent_devices"));
    executed.insert(op(
        "DELETE",
        "/overrides/persistent_devices/by_circuit/{circuit_id}",
    ));
    executed.insert(op(
        "DELETE",
        "/overrides/persistent_devices/by_device/{device_id}",
    ));

    let persistent_body = serde_json::json!({
        "circuit_id": circuit_id,
        "circuit_name": format!("api_test circuit {suffix}"),
        "device_id": device_id,
        "device_name": format!("api_test device {suffix}"),
        "parent_node": "Root",
        "mac": "",
        "ipv4_list": ["100.64.0.251"],
        "ipv6_list": [],
        "download_min_mbps": 1.0,
        "upload_min_mbps": 1.0,
        "download_max_mbps": 10.0,
        "upload_max_mbps": 5.0,
        "comment": "api_test",
        "sqm_override": null
    });
    let resp = client
        .post_json("/overrides/persistent_devices", persistent_body.clone())
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST /overrides/persistent_devices failed: {}",
        resp.status
    );

    let resp = client
        .delete(&format!(
            "/overrides/persistent_devices/by_device/{}",
            urlencoding::encode(&device_id)
        ))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE /overrides/persistent_devices/by_device/{{device_id}} failed: {}",
        resp.status
    );

    // add again and delete by circuit
    let resp = client
        .post_json("/overrides/persistent_devices", persistent_body)
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST /overrides/persistent_devices (2) failed: {}",
        resp.status
    );
    let resp = client
        .delete(&format!(
            "/overrides/persistent_devices/by_circuit/{}",
            urlencoding::encode(&circuit_id)
        ))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE /overrides/persistent_devices/by_circuit/{{circuit_id}} failed: {}",
        resp.status
    );

    // --- Overrides: circuit adjustments + delete by index ---
    executed.insert(op("POST", "/overrides/adjustments/circuit_speed"));
    executed.insert(op("POST", "/overrides/adjustments/device_speed"));
    executed.insert(op("POST", "/overrides/adjustments/remove_circuit"));
    executed.insert(op("POST", "/overrides/adjustments/remove_device"));
    executed.insert(op("POST", "/overrides/adjustments/reparent_circuit"));
    executed.insert(op("DELETE", "/overrides/adjustments/{index}"));

    async fn list_len(client: &ApiClient, path: &str) -> Result<usize> {
        let resp = client.get(path).await?;
        anyhow::ensure!(
            resp.status == StatusCode::OK,
            "GET {path} failed: {}",
            resp.status
        );
        let v: serde_json::Value = serde_json::from_slice(&resp.body)?;
        Ok(v.as_array().map(|a| a.len()).unwrap_or(0))
    }

    let before = list_len(client, "/overrides/adjustments").await?;
    let resp = client
        .post_json(
            "/overrides/adjustments/circuit_speed",
            serde_json::json!({"circuit_id": circuit_id, "min_download_bandwidth": 1.0, "max_download_bandwidth": 2.0, "min_upload_bandwidth": 1.0, "max_upload_bandwidth": 2.0}),
        )
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST circuit_speed failed: {}",
        resp.status
    );
    let resp = client
        .delete(&format!("/overrides/adjustments/{before}"))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE adjustment index failed: {}",
        resp.status
    );

    let before = list_len(client, "/overrides/adjustments").await?;
    let resp = client
        .post_json(
            "/overrides/adjustments/device_speed",
            serde_json::json!({"device_id": device_id, "min_download_bandwidth": 1.0, "max_download_bandwidth": 2.0, "min_upload_bandwidth": 1.0, "max_upload_bandwidth": 2.0}),
        )
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST device_speed failed: {}",
        resp.status
    );
    let resp = client
        .delete(&format!("/overrides/adjustments/{before}"))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE adjustment index failed: {}",
        resp.status
    );

    let before = list_len(client, "/overrides/adjustments").await?;
    let resp = client
        .post_json(
            "/overrides/adjustments/remove_circuit",
            serde_json::json!({"circuit_id": circuit_id}),
        )
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST remove_circuit failed: {}",
        resp.status
    );
    let resp = client
        .delete(&format!("/overrides/adjustments/{before}"))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE adjustment index failed: {}",
        resp.status
    );

    let before = list_len(client, "/overrides/adjustments").await?;
    let resp = client
        .post_json(
            "/overrides/adjustments/remove_device",
            serde_json::json!({"device_id": device_id}),
        )
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST remove_device failed: {}",
        resp.status
    );
    let resp = client
        .delete(&format!("/overrides/adjustments/{before}"))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE adjustment index failed: {}",
        resp.status
    );

    let before = list_len(client, "/overrides/adjustments").await?;
    let resp = client
        .post_json(
            "/overrides/adjustments/reparent_circuit",
            serde_json::json!({"circuit_id": circuit_id, "parent_node": "Root"}),
        )
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST reparent_circuit failed: {}",
        resp.status
    );
    let resp = client
        .delete(&format!("/overrides/adjustments/{before}"))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE adjustment index failed: {}",
        resp.status
    );

    // --- Overrides: network adjustments ---
    executed.insert(op("POST", "/overrides/network_adjustments/site_speed"));
    executed.insert(op("DELETE", "/overrides/network_adjustments/{index}"));

    let before = list_len(client, "/overrides/network_adjustments").await?;
    let resp = client
        .post_json(
            "/overrides/network_adjustments/site_speed",
            serde_json::json!({"site_name": "Root", "download_bandwidth_mbps": 100, "upload_bandwidth_mbps": 100}),
        )
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST network site_speed failed: {}",
        resp.status
    );
    let resp = client
        .delete(&format!("/overrides/network_adjustments/{before}"))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE network adjustment index failed: {}",
        resp.status
    );

    // --- Overrides: UISP bandwidth + routes ---
    executed.insert(op("PUT", "/overrides/uisp/bandwidth"));
    executed.insert(op("DELETE", "/overrides/uisp/bandwidth/{site_name}"));
    executed.insert(op("POST", "/overrides/uisp/routes"));
    executed.insert(op("DELETE", "/overrides/uisp/routes/{index}"));

    let site = "api_test_site";
    let resp = client
        .put_json(
            "/overrides/uisp/bandwidth",
            serde_json::json!({"site_name": site, "down": 10.0, "up": 5.0}),
        )
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "PUT uisp bandwidth failed: {}",
        resp.status
    );

    let resp = client
        .delete(&format!(
            "/overrides/uisp/bandwidth/{}",
            urlencoding::encode(site)
        ))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE uisp bandwidth failed: {}",
        resp.status
    );

    let before = list_len(client, "/overrides/uisp/routes").await?;
    let resp = client
        .post_json(
            "/overrides/uisp/routes",
            serde_json::json!({"from_site": "A", "to_site": "B", "cost": 1}),
        )
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "POST uisp route add failed: {}",
        resp.status
    );
    let resp = client
        .delete(&format!("/overrides/uisp/routes/{before}"))
        .await?;
    anyhow::ensure!(
        resp.status == StatusCode::OK,
        "DELETE uisp route index failed: {}",
        resp.status
    );

    // Coverage check for non-GET endpoints
    let mut missing = Vec::new();
    for op in ops {
        if op.method == "GET" {
            continue;
        }
        if SKIP_ALWAYS
            .iter()
            .any(|(m, p)| *m == op.method && *p == op.path)
        {
            continue;
        }
        if !executed.contains(&op) {
            missing.push(format!("{} {}", op.method, op.path));
        }
    }
    if !missing.is_empty() {
        anyhow::bail!(
            "Mutating suite did not cover some non-GET OpenAPI operations:\n{}",
            missing.join("\n")
        );
    }

    Ok(())
}
