use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
use ip_network::IpNetwork;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};

fn ipv6_overlap(a: &IpNetwork, b: &IpNetwork) -> bool {
    match (a, b) {
        (IpNetwork::V6(a6), IpNetwork::V6(b6)) => {
            let ap_len = a6.netmask() as u32;
            let bp_len = b6.netmask() as u32;
            let minp = ap_len.min(bp_len);
            if minp == 0 {
                return true;
            }
            let addr_a = u128::from_be_bytes(a6.network_address().octets());
            let addr_b = u128::from_be_bytes(b6.network_address().octets());
            let mask: u128 = if minp == 128 {
                !0
            } else {
                (!0u128) << (128 - minp)
            };
            (addr_a & mask) == (addr_b & mask)
        }
        _ => false,
    }
}

fn pretty_net(n: &IpNetwork) -> String {
    match n {
        IpNetwork::V6(v6) => {
            let o = v6.network_address().octets();
            if o[0..10] == [0; 10] && o[10] == 0xff && o[11] == 0xff {
                // IPv4-mapped IPv6
                let v4 = Ipv4Addr::new(o[12], o[13], o[14], o[15]);
                let p = v6.netmask().saturating_sub(96);
                if p >= 32 {
                    format!("{}", v4)
                } else {
                    format!("{}/{}", v4, p)
                }
            } else {
                v6.to_string()
            }
        }
        IpNetwork::V4(v4) => v4.to_string(),
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SearchRequest {
    pub term: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum SearchResult {
    Circuit {
        id: String,
        name: String,
    },
    Device {
        circuit_id: String,
        name: String,
        circuit_name: String,
    },
    Site {
        idx: usize,
        name: String,
    },
}

pub fn search_results(search: SearchRequest) -> Vec<SearchResult> {
    const MAX_RESULTS: usize = 50;
    let mut results: Vec<SearchResult> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new(); // keys like "Device:<circuit_id>:<name>" or "Circuit:<id>" or "Site:<idx>"

    let raw_term = search.term.trim();
    let term_lc = raw_term.to_lowercase();
    let exact_ip: Option<IpAddr> = raw_term.parse::<IpAddr>().ok();
    let looks_like_ip_prefix = raw_term.contains('.') || raw_term.contains(':');

    // Helper to add results with de-dup and cap
    fn push_result(
        results: &mut Vec<SearchResult>,
        seen: &mut HashSet<String>,
        r: SearchResult,
        max_results: usize,
    ) {
        if results.len() >= max_results {
            return;
        }
        let key = match &r {
            SearchResult::Circuit { id, .. } => format!("Circuit:{}", id),
            SearchResult::Device {
                circuit_id, name, ..
            } => format!("Device:{}:{}", circuit_id, name),
            SearchResult::Site { idx, .. } => format!("Site:{}", idx),
        };
        if seen.insert(key) {
            results.push(r);
        }
    }

    // First pass: exact IP matches using the LPM trie
    if let Some(ip) = exact_ip {
        let sd_reader = SHAPED_DEVICES.load();
        let query_v6 = match ip {
            IpAddr::V4(v4) => v4.to_ipv6_mapped(),
            IpAddr::V6(v6) => v6,
        };
        if let Some((net, &idx)) = sd_reader.trie.longest_match(query_v6) {
            if let Some(dev) = sd_reader.devices.get(idx) {
                let name = format!("{} ({})", dev.device_name, pretty_net(&net));
                push_result(
                    &mut results,
                    &mut seen,
                    SearchResult::Device {
                        circuit_id: dev.circuit_id.clone(),
                        name,
                        circuit_name: dev.circuit_name.clone(),
                    },
                    MAX_RESULTS,
                );
            }
        }
    }

    // Second pass: CIDR or IP prefix matches
    if results.len() < MAX_RESULTS && looks_like_ip_prefix && term_lc.len() >= 3 {
        // If term parses as CIDR, match via trie overlap
        if raw_term.contains('/') {
            if let Ok(net) = raw_term.parse::<IpNetwork>() {
                // Normalize to IPv6 network to compare with trie
                let net_v6: Option<IpNetwork> = match net {
                    IpNetwork::V4(v4net) => {
                        let addr = v4net.network_address().to_ipv6_mapped();
                        let pref: u8 = v4net.netmask();
                        let mapped_pref = pref.saturating_add(96);
                        ip_network::Ipv6Network::new(addr, mapped_pref)
                            .ok()
                            .map(IpNetwork::V6)
                    }
                    IpNetwork::V6(v6net) => Some(IpNetwork::V6(v6net)),
                };
                if let Some(query_v6) = net_v6.as_ref() {
                    let sd_reader = SHAPED_DEVICES.load();
                    for (n, &idx) in sd_reader.trie.iter() {
                        if results.len() >= MAX_RESULTS {
                            break;
                        }
                        if ipv6_overlap(&n, query_v6) {
                            if let Some(dev) = sd_reader.devices.get(idx) {
                                let name = format!("{} ({})", dev.device_name, pretty_net(&n));
                                push_result(
                                    &mut results,
                                    &mut seen,
                                    SearchResult::Device {
                                        circuit_id: dev.circuit_id.clone(),
                                        name,
                                        circuit_name: dev.circuit_name.clone(),
                                    },
                                    MAX_RESULTS,
                                );
                            }
                        }
                    }
                }
            }
        } else {
            // Fallback: textual prefix (e.g., "10.1.")
            let sd_reader = SHAPED_DEVICES.load();
            for (n, &idx) in sd_reader.trie.iter() {
                if results.len() >= MAX_RESULTS {
                    break;
                }
                let s = pretty_net(&n);
                if s.starts_with(raw_term) {
                    if let Some(dev) = sd_reader.devices.get(idx) {
                        let name = format!("{} ({})", dev.device_name, s);
                        push_result(
                            &mut results,
                            &mut seen,
                            SearchResult::Device {
                                circuit_id: dev.circuit_id.clone(),
                                name,
                                circuit_name: dev.circuit_name.clone(),
                            },
                            MAX_RESULTS,
                        );
                    }
                }
            }
        }
    }

    // Third pass: Circuit/Device name substring matches
    if results.len() < MAX_RESULTS && term_lc.len() >= 3 {
        let sd_reader = SHAPED_DEVICES.load();
        for sd in sd_reader.devices.iter() {
            if results.len() >= MAX_RESULTS {
                break;
            }
            let circuit_name_lc = sd.circuit_name.to_lowercase();
            if circuit_name_lc.contains(&term_lc) {
                push_result(
                    &mut results,
                    &mut seen,
                    SearchResult::Circuit {
                        id: sd.circuit_id.clone(),
                        name: sd.circuit_name.clone(),
                    },
                    MAX_RESULTS,
                );
            }
            if results.len() >= MAX_RESULTS {
                break;
            }
            let device_name_lc = sd.device_name.to_lowercase();
            if device_name_lc.contains(&term_lc) {
                push_result(
                    &mut results,
                    &mut seen,
                    SearchResult::Device {
                        circuit_id: sd.circuit_id.clone(),
                        name: sd.device_name.clone(),
                        circuit_name: sd.circuit_name.clone(),
                    },
                    MAX_RESULTS,
                );
            }
        }
    }

    // Fourth pass: Site name substring matches
    if results.len() < MAX_RESULTS && term_lc.len() >= 3 {
        let net_reader = NETWORK_JSON.read();
        for (idx, n) in net_reader.get_nodes_when_ready().iter().enumerate() {
            if results.len() >= MAX_RESULTS {
                break;
            }
            if n.name.to_lowercase().contains(&term_lc) {
                push_result(
                    &mut results,
                    &mut seen,
                    SearchResult::Site {
                        idx,
                        name: n.name.clone(),
                    },
                    MAX_RESULTS,
                );
            }
        }
    }

    results
}
