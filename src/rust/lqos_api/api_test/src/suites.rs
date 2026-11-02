use anyhow::{Context as _, Result};
use reqwest::StatusCode;

use crate::{client::ApiClient, context, openapi};

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
                StatusCode::FORBIDDEN => {
                    anyhow::bail!(
                        "API policy denied access (403): {}",
                        String::from_utf8_lossy(&resp.body)
                    );
                }
                StatusCode::UNAUTHORIZED => {
                    anyhow::bail!("API returned 401 Unauthorized. Check the x-bearer credential in LQOS_API_KEY.");
                }
                _ => {
                    // lqosd bus may not be ready yet.
                }
            },
            Err(_) => {
                // Server may not be up yet.
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

fn encode_path_segment(s: &str) -> String {
    urlencoding::encode(s).to_string()
}

fn fill_path_template(template: &str, ctx: &context::ContextValues) -> Result<String> {
    let mut out = String::new();
    let mut rest = template;
    loop {
        let Some(start) = rest.find('{') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            anyhow::bail!("Invalid path template (missing }}): {template}");
        };
        let key = &after[..end];
        let value = match key {
            "seconds" => Some("300".to_string()),
            "start" => Some("0".to_string()),
            "end" => Some("10".to_string()),
            "n" => Some("10".to_string()),
            "parent" => Some("0".to_string()),
            "flow_type" => Some("RateEstimate".to_string()),
            "asn" => ctx.asn.clone().or(Some("0".to_string())),
            "iso" => ctx.iso.clone().or(Some("US".to_string())),
            "protocol" => ctx.protocol.clone().or(Some("tcp".to_string())),
            "circuit_id" => ctx.circuit_id.clone(),
            "target" => ctx.circuit_id.clone(),
            "ip" => ctx.ip_address.clone(),
            "ip_address" => ctx.ip_address.clone(),
            "node_name" => ctx.node_name.clone().or(Some("Root".to_string())),
            "device_id" => ctx.device_id.clone(),
            "id" => ctx.urgent_id.clone().or(Some("0".to_string())),
            other => anyhow::bail!("No value mapping for path parameter '{other}'"),
        }
        .with_context(|| format!("missing required context value for {{{key}}}"))?;

        out.push_str(&encode_path_segment(&value));
        rest = &after[end + 1..];
    }
    Ok(out)
}

fn response_is_json(headers: &reqwest::header::HeaderMap) -> bool {
    let Some(ct) = headers.get(reqwest::header::CONTENT_TYPE) else {
        return false;
    };
    let Ok(ct) = ct.to_str() else { return false; };
    ct.starts_with("application/json")
}

fn validate_ok_response(resp: &crate::client::ApiResponse) -> Result<()> {
    if !resp.status.is_success() {
        anyhow::bail!(
            "HTTP {}: {}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
        );
    }

    if resp.body.is_empty() {
        return Ok(());
    }

    if response_is_json(&resp.headers) {
        let _v: serde_json::Value = serde_json::from_slice(&resp.body)
            .context("response body was not valid json")?;
        return Ok(());
    }

    // text/plain or unknown content-type
    let text = String::from_utf8_lossy(&resp.body);
    if text.trim().is_empty() {
        anyhow::bail!("response body was empty/whitespace");
    }
    Ok(())
}

pub async fn run_smoke(client: &ApiClient) -> Result<()> {
    wait_until_ready(client).await?;

    let endpoints = [
        "/openapi.json",
        "/health",
        "/version",
        "/cpu_usage",
        "/ram_usage",
        "/system_stats_health",
        "/current_throughput",
        "/host_counts",
        "/network_tree",
        "/all_circuits",
        "/warnings",
    ];

    let mut failures = 0usize;
    for path in endpoints {
        let resp = client.get(path).await;
        match resp {
            Ok(resp) => {
                let ok = resp.status == StatusCode::OK;
                println!(
                    "GET {path} -> {}{}",
                    resp.status.as_u16(),
                    if ok { " (ok)" } else { " (fail)" }
                );
                if ok {
                    if let Err(e) = validate_ok_response(&resp) {
                        eprintln!("  validation failed: {e}");
                        failures += 1;
                    }
                } else {
                    failures += 1;
                }
            }
            Err(e) => {
                eprintln!("GET {path} -> request error: {e}");
                failures += 1;
            }
        }
    }

    if failures > 0 {
        anyhow::bail!("{failures} request(s) failed");
    }
    Ok(())
}

pub async fn run_full(client: &ApiClient) -> Result<()> {
    wait_until_ready(client).await?;

    let ops = openapi::fetch_operations(client).await?;
    let ctx = context::discover(client).await?;

    let mut failures = 0usize;
    let mut tested = 0usize;
    let mut skipped = 0usize;

    for op in ops {
        if op.method != "GET" {
            skipped += 1;
            continue;
        }

        let mut path = fill_path_template(&op.path, &ctx)
            .with_context(|| format!("materialize GET {}", op.path))?;

        // Provide a default search term for /search.
        if path == "/search" {
            let term = ctx
                .ip_address
                .clone()
                .unwrap_or_else(|| "100.64.0.1".to_string());
            path = format!("/search?term={}", urlencoding::encode(&term));
        }

        let resp = client.get(&path).await;
        match resp {
            Ok(resp) => {
                tested += 1;
                let ok = resp.status == StatusCode::OK;
                println!(
                    "GET {} -> {}{}",
                    op.path,
                    resp.status.as_u16(),
                    if ok { " (ok)" } else { " (fail)" }
                );
                if ok {
                    if let Err(e) = validate_ok_response(&resp) {
                        eprintln!("  validation failed: {e}");
                        failures += 1;
                    }
                } else {
                    eprintln!("  body: {}", String::from_utf8_lossy(&resp.body));
                    failures += 1;
                }
            }
            Err(e) => {
                tested += 1;
                eprintln!("GET {} -> request error: {e}", op.path);
                failures += 1;
            }
        }
    }

    println!(
        "Full suite summary: tested={tested}, skipped_non_get={skipped}, failures={failures}"
    );

    if failures > 0 {
        anyhow::bail!("{failures} request(s) failed");
    }
    Ok(())
}
