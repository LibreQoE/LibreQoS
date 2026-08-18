use ip_network::IpNetwork;
use ip_network::{Ipv4Network, Ipv6Network};
use lqos_utils::XdpIpAddress;
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

fn dynamic_longest_match_for_ip(
    ip: IpAddr,
    dynamic: &[lqos_network_devices::DynamicCircuit],
) -> Option<(IpNetwork, &lqos_config::ShapedDevice)> {
    let mut best: Option<(IpNetwork, &lqos_config::ShapedDevice, u8)> = None;

    for circuit in dynamic {
        let dev = &circuit.shaped;
        match ip {
            IpAddr::V4(ip4) => {
                for (addr, prefix) in &dev.ipv4 {
                    let Some(prefix_u8) = u8::try_from(*prefix).ok() else {
                        continue;
                    };
                    let Ok(net) = Ipv4Network::new(*addr, prefix_u8) else {
                        continue;
                    };
                    if !net.contains(ip4) {
                        continue;
                    }
                    if best.as_ref().map(|(_, _, p)| *p).unwrap_or(0) < prefix_u8 {
                        best = Some((IpNetwork::V4(net), dev, prefix_u8));
                    }
                }
            }
            IpAddr::V6(ip6) => {
                for (addr, prefix) in &dev.ipv6 {
                    let Some(prefix_u8) = u8::try_from(*prefix).ok() else {
                        continue;
                    };
                    let Ok(net) = Ipv6Network::new(*addr, prefix_u8) else {
                        continue;
                    };
                    if !net.contains(ip6) {
                        continue;
                    }
                    if best.as_ref().map(|(_, _, p)| *p).unwrap_or(0) < prefix_u8 {
                        best = Some((IpNetwork::V6(net), dev, prefix_u8));
                    }
                }
            }
        }
    }

    best.map(|(net, dev, _)| (net, dev))
}

pub fn search_results(search: SearchRequest) -> Vec<SearchResult> {
    const MAX_RESULTS: usize = 50;
    let mut results: Vec<SearchResult> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new(); // keys like "Device:<circuit_id>:<name>" or "Circuit:<id>" or "Site:<idx>"

    let raw_term = search.term.trim();
    let term_lc = raw_term.to_lowercase();
    let exact_ip: Option<IpAddr> = raw_term.parse::<IpAddr>().ok();
    let looks_like_ip_prefix = raw_term.contains('.') || raw_term.contains(':');
    let catalog = lqos_network_devices::shaped_devices_catalog();
    let dynamic_snapshot = lqos_network_devices::dynamic_circuits_snapshot();

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
        let xdp_ip = XdpIpAddress::from_ip(ip);
        if let Some((net, dev)) = catalog.device_longest_match_for_ip(&xdp_ip) {
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
        } else if let Some((net, dev)) = dynamic_longest_match_for_ip(ip, dynamic_snapshot.as_ref())
        {
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
                    for (n, dev) in catalog.iter_ip_mappings() {
                        if results.len() >= MAX_RESULTS {
                            break;
                        }
                        if ipv6_overlap(&n, query_v6) {
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
                    for circuit in dynamic_snapshot.iter() {
                        if results.len() >= MAX_RESULTS {
                            break;
                        }
                        let dev = &circuit.shaped;
                        for (addr, prefix) in &dev.ipv4 {
                            if results.len() >= MAX_RESULTS {
                                break;
                            }
                            let Some(prefix_u8) = u8::try_from(*prefix).ok() else {
                                continue;
                            };
                            let Ok(v4net) = Ipv4Network::new(*addr, prefix_u8) else {
                                continue;
                            };
                            let addr = v4net.network_address().to_ipv6_mapped();
                            let mapped_pref = prefix_u8.saturating_add(96);
                            let Ok(v6net) = Ipv6Network::new(addr, mapped_pref) else {
                                continue;
                            };
                            let n = IpNetwork::V6(v6net);
                            if ipv6_overlap(&n, query_v6) {
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
                        for (addr, prefix) in &dev.ipv6 {
                            if results.len() >= MAX_RESULTS {
                                break;
                            }
                            let Some(prefix_u8) = u8::try_from(*prefix).ok() else {
                                continue;
                            };
                            let Ok(v6net) = Ipv6Network::new(*addr, prefix_u8) else {
                                continue;
                            };
                            let n = IpNetwork::V6(v6net);
                            if ipv6_overlap(&n, query_v6) {
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
            for (n, dev) in catalog.iter_ip_mappings() {
                if results.len() >= MAX_RESULTS {
                    break;
                }
                let s = pretty_net(&n);
                if s.starts_with(raw_term) {
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
            for circuit in dynamic_snapshot.iter() {
                if results.len() >= MAX_RESULTS {
                    break;
                }
                let dev = &circuit.shaped;
                for (addr, prefix) in &dev.ipv4 {
                    if results.len() >= MAX_RESULTS {
                        break;
                    }
                    let Some(prefix_u8) = u8::try_from(*prefix).ok() else {
                        continue;
                    };
                    let Ok(net) = Ipv4Network::new(*addr, prefix_u8) else {
                        continue;
                    };
                    let n = IpNetwork::V4(net);
                    let s = pretty_net(&n);
                    if s.starts_with(raw_term) {
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
                for (addr, prefix) in &dev.ipv6 {
                    if results.len() >= MAX_RESULTS {
                        break;
                    }
                    let Some(prefix_u8) = u8::try_from(*prefix).ok() else {
                        continue;
                    };
                    let Ok(net) = Ipv6Network::new(*addr, prefix_u8) else {
                        continue;
                    };
                    let n = IpNetwork::V6(net);
                    let s = pretty_net(&n);
                    if s.starts_with(raw_term) {
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
        for sd in catalog.iter_devices() {
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

        for circuit in dynamic_snapshot.iter() {
            if results.len() >= MAX_RESULTS {
                break;
            }
            let sd = &circuit.shaped;
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
        lqos_network_devices::with_network_json_read(|net_reader| {
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
        });
    }

    results
}
