use anyhow::Result;

use crate::client::ApiClient;

#[derive(Clone, Debug, Default)]
pub struct ContextValues {
    pub circuit_id: Option<String>,
    pub device_id: Option<String>,
    pub ip_address: Option<String>,
    pub node_name: Option<String>,
    pub asn: Option<String>,
    pub iso: Option<String>,
    pub protocol: Option<String>,
    pub urgent_id: Option<String>,
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|v| {
        let t = v.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    })
}

pub async fn discover(client: &ApiClient) -> Result<ContextValues> {
    let mut ctx = ContextValues::default();

    ctx.circuit_id = env_opt("TEST_CIRCUIT");
    ctx.device_id = env_opt("TEST_DEVICE");
    ctx.ip_address = env_opt("TEST_IP");
    ctx.node_name = env_opt("TEST_NODE");

    // Try to discover circuit_id from /all_circuits
    if ctx.circuit_id.is_none() {
        if let Ok(resp) = client.get("/all_circuits").await {
            if resp.status.is_success() {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                    if let Some(arr) = v.as_array() {
                        for item in arr {
                            if let Some(id) = item.get("circuit_id").and_then(|x| x.as_str()) {
                                let id = id.trim();
                                if !id.is_empty() {
                                    ctx.circuit_id = Some(id.to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Try to discover device_id and an IP from /shaped_devices
    if ctx.device_id.is_none() || ctx.ip_address.is_none() {
        if let Ok(resp) = client.get("/shaped_devices").await {
            if resp.status.is_success() {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                    if let Some(arr) = v.as_array() {
                        for item in arr {
                            if ctx.device_id.is_none() {
                                if let Some(did) = item.get("device_id").and_then(|x| x.as_str()) {
                                    let did = did.trim();
                                    if !did.is_empty() {
                                        ctx.device_id = Some(did.to_string());
                                    }
                                }
                            }
                            if ctx.ip_address.is_none() {
                                if let Some(ipv4) = item.get("ipv4").and_then(|x| x.as_array()) {
                                    for entry in ipv4 {
                                        // entry is [ "ip", prefix ]
                                        if let Some(ip) = entry.get(0).and_then(|x| x.as_str()) {
                                            let ip = ip.trim();
                                            if !ip.is_empty() {
                                                ctx.ip_address = Some(ip.to_string());
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            if ctx.device_id.is_some() && ctx.ip_address.is_some() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // Try to discover node_name from /network_tree
    if ctx.node_name.is_none() {
        if let Ok(resp) = client.get("/network_tree").await {
            if resp.status.is_success() {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                    if let Some(arr) = v.as_array() {
                        for item in arr {
                            if let Some(name) = item.get("name").and_then(|x| x.as_str()) {
                                let name = name.trim();
                                if !name.is_empty() {
                                    ctx.node_name = Some(name.to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Flow explorer parameters
    if let Ok(resp) = client.get("/flow_explorer/asns").await {
        if resp.status.is_success() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                if let Some(arr) = v.as_array() {
                    if let Some(first) = arr.first() {
                        if let Some(asn) = first.get("asn").and_then(|x| x.as_i64()) {
                            ctx.asn = Some(asn.to_string());
                        }
                    }
                }
            }
        }
    }
    if let Ok(resp) = client.get("/flow_explorer/countries").await {
        if resp.status.is_success() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                if let Some(arr) = v.as_array() {
                    if let Some(first) = arr.first() {
                        if let Some(iso) = first.get("iso").and_then(|x| x.as_str()) {
                            let iso = iso.trim();
                            if !iso.is_empty() {
                                ctx.iso = Some(iso.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    if let Ok(resp) = client.get("/flow_explorer/protocols").await {
        if resp.status.is_success() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                if let Some(arr) = v.as_array() {
                    if let Some(first) = arr.first() {
                        if let Some(proto) = first.get("protocol").and_then(|x| x.as_str()) {
                            let proto = proto.trim();
                            if !proto.is_empty() {
                                ctx.protocol = Some(proto.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Urgent ID (optional)
    if let Ok(resp) = client.get("/urgent").await {
        if resp.status.is_success() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
                    if let Some(first) = items.first() {
                        if let Some(id) = first.get("id").and_then(|x| x.as_u64()) {
                            ctx.urgent_id = Some(id.to_string());
                        }
                    }
                }
            }
        }
    }

    Ok(ctx)
}

