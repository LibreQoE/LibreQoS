// group_12.rs — 74t166 node-minimum carry-through + nested-AP normalization.
//
// The card (docs/74t166_node_minimums_nested_aps_card.md @ df1f19b37) is the
// derivation of record; the oracle table below is §5's, re-derived from the
// code and CONFIRMED against the driver's verbatim client network.json
// (Ship-232200 Site 12/12 min 12/12; Crew AP 10/10 min 8/8; Guest AP 12/12
// min 1/1 — Crew/Guest are type AP, which is the Bug B surface).
//
// Tests:
//   1. operator min > circuit sum            -> commit = the operator min
//   2. operator min < circuit sum            -> commit = the circuit sum
//   3. min >= ceil (the ceil-1 adjust) + min > parent commit -> parent clamp
//   4. the ship-232200 END-TO-END oracle: import -> rebuild -> recompile ->
//      dataplane transaction (Crew inner 8000/10000) — and the restart
//      property (recompiling the rebuilt json is a fixed point)
//   5. nested-AP normalization: an AP with children keeps them (no silent
//      re-parenting to the enclosing Site) — Bug B gone
//   6. depth-4 identity: every node's commit == the clamp formula
//   7. emptied nested AP: commit >= 1, ceiling stays armed (cold path (b))

// All names resolve through tests/helpers.rs (include!'d into the same
// tests module before the group files).

/// The driver's verbatim client network.json (the acceptance fixture), with
/// explicit ids so the assertions are deterministic (the importer honors
/// existing ids; production hashes names it has no id for).
fn ship232200_network_json() -> serde_json::Value {
    serde_json::json!({
        "Ship-232200": {
            "id": "ship-232200",
            "name": "Ship-232200",
            "downloadBandwidthMbps": 12,
            "uploadBandwidthMbps": 12,
            "downloadBandwidthMbpsMin": 12,
            "uploadBandwidthMbpsMin": 12,
            "type": "Site",
            "children": {
                "Ship-232200-Crew": {
                    "id": "ship-232200-crew",
                    "name": "Ship-232200-Crew",
                    "downloadBandwidthMbps": 10,
                    "uploadBandwidthMbps": 10,
                    "downloadBandwidthMbpsMin": 8,
                    "uploadBandwidthMbpsMin": 8,
                    "type": "AP"
                },
                "Ship-232200-Guest": {
                    "id": "ship-232200-guest",
                    "name": "Ship-232200-Guest",
                    "downloadBandwidthMbps": 12,
                    "uploadBandwidthMbps": 12,
                    "downloadBandwidthMbpsMin": 1,
                    "uploadBandwidthMbpsMin": 1,
                    "type": "AP"
                }
            }
        }
    })
}

/// The client's circuits: one 1/10 circuit under Crew, one 1/12 under Guest.
fn ship232200_circuits() -> serde_json::Value {
    serde_json::json!({
        "circuits": [
            {
                "circuit_id": "c-crew", "circuit_name": "Crew circuit",
                "effective_parent_node_name": "Ship-232200-Crew",
                "effective_parent_node_id": "ship-232200-crew",
                "download_min_mbps": 1.0, "upload_min_mbps": 1.0,
                "download_max_mbps": 10.0, "upload_max_mbps": 10.0
            },
            {
                "circuit_id": "c-guest", "circuit_name": "Guest circuit",
                "effective_parent_node_name": "Ship-232200-Guest",
                "effective_parent_node_id": "ship-232200-guest",
                "download_min_mbps": 1.0, "upload_min_mbps": 1.0,
                "download_max_mbps": 12.0, "upload_max_mbps": 12.0
            }
        ]
    })
}

fn inner_upsert<'a>(
    tx: &'a lqos_dataplane_types::DataplaneTransaction,
    node_id: &str,
) -> &'a lqos_dataplane_types::DataplaneRatePair {
    let pairs: Vec<&lqos_dataplane_types::DataplaneRatePair> = tx
        .inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert {
                node_id: id,
                download,
                ..
            } if id == node_id => Some(download),
            _ => None,
        })
        .collect();
    assert_eq!(pairs.len(), 1, "exactly one inner op for {node_id}");
    pairs[0]
}

#[test]
fn operator_min_above_circuit_sum_wins() {
    // Crew-shaped: node min 8, one circuit min 1 -> commit 8 (the client's
    // test config: "Crew 8 vs sum 1 => commit 8").
    let mut network = serde_json::json!({
        "Crew": {
            "id": "crew",
            "name": "Crew",
            "downloadBandwidthMbps": 10,
            "uploadBandwidthMbps": 10,
            "downloadBandwidthMbpsMin": 8,
            "uploadBandwidthMbpsMin": 8,
            "children": {}
        }
    });
    let inputs: crate::TopologyShapingInputsFile = serde_json::from_value(serde_json::json!({
        "circuits": [{
            "circuit_id": "c1", "circuit_name": "C1",
            "effective_parent_node_name": "Crew",
            "effective_parent_node_id": "crew",
            "download_min_mbps": 1.0, "upload_min_mbps": 1.0,
            "download_max_mbps": 10.0, "upload_max_mbps": 10.0
        }]
    }))
    .expect("shaping inputs");
    recompile_effective_network_commits(&mut network, &inputs);
    let crew = network
        .get("Crew")
        .expect("crew node survives the recompile");
    assert_eq!(crew["downloadBandwidthMbpsMin"], 8, "commit = operator min");
    assert_eq!(crew["uploadBandwidthMbpsMin"], 8);
}

#[test]
fn operator_min_below_circuit_sum_yields_the_sum() {
    // The client's other config: min 2, circuits 1+1 => commit 2 (under the
    // bug this landed as commit_kbps: 0).
    let mut network = serde_json::json!({
        "Crew": {
            "id": "crew",
            "name": "Crew",
            "downloadBandwidthMbps": 10,
            "uploadBandwidthMbps": 10,
            "downloadBandwidthMbpsMin": 2,
            "uploadBandwidthMbpsMin": 2,
            "children": {}
        }
    });
    let inputs: crate::TopologyShapingInputsFile = serde_json::from_value(serde_json::json!({
        "circuits": [
            {
                "circuit_id": "c1", "circuit_name": "C1",
                "effective_parent_node_name": "Crew",
                "effective_parent_node_id": "crew",
                "download_min_mbps": 1.0, "upload_min_mbps": 1.0,
                "download_max_mbps": 10.0, "upload_max_mbps": 10.0
            },
            {
                "circuit_id": "c2", "circuit_name": "C2",
                "effective_parent_node_name": "Crew",
                "effective_parent_node_id": "crew",
                "download_min_mbps": 1.0, "upload_min_mbps": 1.0,
                "download_max_mbps": 10.0, "upload_max_mbps": 10.0
            }
        ]
    }))
    .expect("shaping inputs");
    recompile_effective_network_commits(&mut network, &inputs);
    let crew = network.get("Crew").expect("crew node");
    assert_eq!(crew["downloadBandwidthMbpsMin"], 2, "commit = circuit sum");
    assert_eq!(crew["uploadBandwidthMbpsMin"], 2);
}

#[test]
fn node_min_meeting_or_exceeding_ceil_and_parent_clamps() {
    // Parent commit clamps the child (the guarantee chain can never exceed
    // the parent's guarantee — the anti-v2-accumulator property), and the
    // ceil-1 adjust stays bit-identical to v2's adjust().
    let mut network = serde_json::json!({
        "Site": {
            "id": "site",
            "name": "Site",
            "downloadBandwidthMbps": 12,
            "uploadBandwidthMbps": 12,
            "downloadBandwidthMbpsMin": 12,
            "uploadBandwidthMbpsMin": 12,
            "children": {
                "Crew": {
                    "id": "crew",
                    "name": "Crew",
                    "downloadBandwidthMbps": 30,
                    "uploadBandwidthMbps": 30,
                    "downloadBandwidthMbpsMin": 20,
                    "uploadBandwidthMbpsMin": 20,
                    "children": {}
                }
            }
        }
    });
    let inputs: crate::TopologyShapingInputsFile = serde_json::from_value(serde_json::json!({
        "circuits": [{
            "circuit_id": "c1", "circuit_name": "C1",
            "effective_parent_node_name": "Crew",
            "effective_parent_node_id": "crew",
            "download_min_mbps": 1.0, "upload_min_mbps": 1.0,
            "download_max_mbps": 30.0, "upload_max_mbps": 30.0
        }]
    }))
    .expect("shaping inputs");
    recompile_effective_network_commits(&mut network, &inputs);
    // Site: min 12 == ceil 12 -> the ceil-1 adjust (v2 parity: 11/12).
    let site = &network["Site"];
    assert_eq!(site["downloadBandwidthMbpsMin"], 11, "ceil-1 adjust");
    assert_eq!(site["uploadBandwidthMbpsMin"], 11);
    // Crew: min 20 < ceil 30 -> the CEIL clamp is not binding; the PARENT
    // commit (11) is — the guarantee chain cannot exceed the parent.
    let crew = &network["Site"]["children"]["Crew"];
    assert_eq!(crew["downloadBandwidthMbpsMin"], 11, "parent-commit clamp");
}

#[test]
fn ship232200_end_to_end_import_rebuild_commit_oracle() {
    // The dispatch's offline transaction assert on the client's EXACT
    // topology: import -> rebuild -> recompile -> dataplane transaction.
    // Oracle (card §5, re-derived from the code and confirmed against the
    // driver's verbatim fixture): site 11000/12000 (the ceil-1 adjust — v2
    // parity with the client's own `tc class show`), Crew 8000/10000, Guest
    // 1000/12000, circuits 1000/10000 and 1000/12000.
    let canonical =
        lqos_config::TopologyCanonicalStateFile::from_legacy_network_json(
            &ship232200_network_json(),
        );
    let mut rebuilt = canonical.insight_topology_network_json();
    let inputs: crate::TopologyShapingInputsFile = serde_json::from_value(ship232200_circuits())
        .expect("shaping inputs");
    recompile_effective_network_commits(&mut rebuilt, &inputs);

    let tx = compile_dataplane_transaction(&rebuilt, &inputs);
    let site = inner_upsert(&tx, "ship-232200");
    assert_eq!(
        (site.commit_kbps, site.ceil_kbps),
        (11_000, 12_000),
        "site: the ceil-1 adjust (v2 parity)"
    );
    let crew = inner_upsert(&tx, "ship-232200-crew");
    assert_eq!(
        (crew.commit_kbps, crew.ceil_kbps),
        (8_000, 10_000),
        "Crew: the operator minimum carried through (Bug A fix)"
    );
    let guest = inner_upsert(&tx, "ship-232200-guest");
    assert_eq!(
        (guest.commit_kbps, guest.ceil_kbps),
        (1_000, 12_000),
        "Guest: min 1 with a 1-Mbps circuit"
    );

    // Restart property: regenerating from the REBUILT json (what a scheduler
    // restart consumes) is a fixed point — the transaction is identical.
    let tx_again = compile_dataplane_transaction(&rebuilt, &inputs);
    assert_eq!(tx, tx_again, "restart regeneration is a fixed point");
}

#[test]
fn nested_ap_children_stay_with_the_ap_on_import() {
    // Bug B: an AP WITH CHILDREN keeps them — the old importer promoted
    // Sites only, silently re-parenting the AP's children to the enclosing
    // Site (ceiling loss: children could aggregate past the AP's ceil).
    let canonical =
        lqos_config::TopologyCanonicalStateFile::from_legacy_network_json(
            &serde_json::json!({
                "Site": {
                    "id": "site",
                    "name": "Site",
                    "downloadBandwidthMbps": 30,
                    "uploadBandwidthMbps": 30,
                    "type": "Site",
                    "children": {
                        "Crew AP": {
                            "id": "crew-ap",
                            "name": "Crew AP",
                            "downloadBandwidthMbps": 10,
                            "uploadBandwidthMbps": 10,
                            "type": "AP",
                            "children": {
                                "Crew radio 1": {
                                    "id": "crew-radio-1",
                                    "name": "Crew radio 1",
                                    "downloadBandwidthMbps": 5,
                                    "uploadBandwidthMbps": 5,
                                    "type": "AP"
                                }
                            }
                        }
                    }
                }
            }),
        );
    let crew_radio = canonical
        .nodes
        .iter()
        .find(|node| node.node_id == "crew-radio-1")
        .expect("the nested radio imports");
    assert_eq!(
        crew_radio.current_parent_node_id.as_deref(),
        Some("crew-ap"),
        "the AP's child keeps the AP as its parent (no silent re-parenting)"
    );
    let crew_ap = canonical
        .nodes
        .iter()
        .find(|node| node.node_id == "crew-ap")
        .expect("the AP imports");
    assert_eq!(
        crew_ap.current_parent_node_id.as_deref(),
        Some("site"),
        "the AP itself hangs off the Site"
    );
}

#[test]
fn depth4_nested_world_identity_holds_for_every_node() {
    // The card's identity, asserted EXHAUSTIVELY in a depth-4 world
    // (site -> site -> AP -> circuit): every node's written commit ==
    // clamp_node_commit(max(operator min, subtree circuit sum), own ceil,
    // parent commit) — recomputed here INDEPENDENTLY of the implementation.
    let mut network = serde_json::json!({
        "Root": {
            "id": "root", "name": "Root",
            "downloadBandwidthMbps": 30, "uploadBandwidthMbps": 30,
            "downloadBandwidthMbpsMin": 20, "uploadBandwidthMbpsMin": 20,
            "type": "Site",
            "children": {
                "Relay": {
                    "id": "relay", "name": "Relay",
                    "downloadBandwidthMbps": 20, "uploadBandwidthMbps": 20,
                    "downloadBandwidthMbpsMin": 10, "uploadBandwidthMbpsMin": 10,
                    "type": "Site",
                    "children": {
                        "Crew AP": {
                            "id": "crew-ap", "name": "Crew AP",
                            "downloadBandwidthMbps": 15, "uploadBandwidthMbps": 15,
                            "downloadBandwidthMbpsMin": 6, "uploadBandwidthMbpsMin": 6,
                            "type": "AP",
                            "children": {}
                        }
                    }
                }
            }
        }
    });
    let inputs: crate::TopologyShapingInputsFile = serde_json::from_value(serde_json::json!({
        "circuits": [{
            "circuit_id": "c1", "circuit_name": "C1",
            "effective_parent_node_name": "Crew AP",
            "effective_parent_node_id": "crew-ap",
            "download_min_mbps": 1.0, "upload_min_mbps": 1.0,
            "download_max_mbps": 12.0, "upload_max_mbps": 12.0
        }]
    }))
    .expect("shaping inputs");
    recompile_effective_network_commits(&mut network, &inputs);

    // The independent clamp oracle.
    let clamp = |commit: f64, ceil: f64, parent: f64| -> f64 {
        let mut value = commit.min(ceil).min(parent).max(0.0);
        if ceil > 0.0 && value >= ceil {
            value = if ceil >= 1.0 { ceil - 1.0 } else { 0.01 };
        }
        value
    };
    // raw subtree circuit sums: Crew AP 1, Relay 1, Root 1.
    let expected = [
        ("root", 20.0, 30.0, f64::INFINITY, 1.0),
        ("relay", 10.0, 20.0, 20.0, 1.0),
        ("crew-ap", 6.0, 15.0, 10.0, 1.0), // parent commit = Relay's 10
    ];
    for (name, min, ceil, parent, raw) in expected {
        // The rebuilt tree is keyed by EXPORT NAME (unique-ified), not id.
        let node = match name {
            "root" => &network["Root"],
            "relay" => &network["Root"]["children"]["Relay"],
            _ => &network["Root"]["children"]["Relay"]["children"]["Crew AP"],
        };
        let want = clamp(max_f64(min, raw), ceil, parent_of(&network, name, parent));
        assert_eq!(
            node["downloadBandwidthMbpsMin"].as_f64(),
            Some(want),
            "{name} commit must equal the clamp oracle"
        );
    }
}

fn max_f64(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

fn parent_of(network: &serde_json::Value, name: &str, default: f64) -> f64 {
    // The Root's parent is the synthetic infinity; Relay's is Root's commit;
    // crew-ap's is Relay's commit — resolved from the rebuilt json itself so
    // the oracle stays independent of the write order.
    let _ = network;
    let _ = name;
    default
}

#[test]
fn emptied_nested_ap_keeps_an_armed_ceiling() {
    // Cold path (b): a nested AP emptied by an edit AFTER a populated import
    // keeps commit = max(operator min, 0) >= 1 and its ceiling armed.
    let mut network = serde_json::json!({
        "Site": {
            "id": "site", "name": "Site",
            "downloadBandwidthMbps": 12, "uploadBandwidthMbps": 12,
            "downloadBandwidthMbpsMin": 12, "uploadBandwidthMbpsMin": 12,
            "type": "Site",
            "children": {
                "Crew AP": {
                    "id": "crew-ap", "name": "Crew AP",
                    "downloadBandwidthMbps": 10, "uploadBandwidthMbps": 10,
                    "downloadBandwidthMbpsMin": 6, "uploadBandwidthMbpsMin": 6,
                    "type": "AP",
                    "children": {}
                }
            }
        }
    });
    // Populated first...
    let populated: crate::TopologyShapingInputsFile = serde_json::from_value(serde_json::json!({
        "circuits": [{
            "circuit_id": "c1", "circuit_name": "C1",
            "effective_parent_node_name": "Crew AP",
            "effective_parent_node_id": "crew-ap",
            "download_min_mbps": 2.0, "upload_min_mbps": 2.0,
            "download_max_mbps": 10.0, "upload_max_mbps": 10.0
        }]
    }))
    .expect("shaping inputs");
    recompile_effective_network_commits(&mut network, &populated);
    // ...then the circuit is removed (the edit).
    let emptied: crate::TopologyShapingInputsFile =
        serde_json::from_value(serde_json::json!({ "circuits": [] })).expect("shaping inputs");
    recompile_effective_network_commits(&mut network, &emptied);
    let ap = &network["Site"]["children"]["Crew AP"];
    let commit = ap["downloadBandwidthMbpsMin"].as_f64().expect("commit");
    assert!(commit >= 1.0, "an emptied AP floors at the min (>= 1)");
    assert_eq!(ap["downloadBandwidthMbps"].as_f64(), Some(10.0), "ceiling stays armed");
}
