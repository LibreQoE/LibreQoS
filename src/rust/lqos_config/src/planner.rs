//! Shared planning helpers for physical queue placement.
//!
//! This module provides a reusable top-level CPU/bin planner so both the Python shaper build path
//! and future Bakery/TreeGuard runtime replans can use the same assignment logic.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A top-level item to be assigned to a shaping queue/bin.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TopLevelPlannerItem {
    /// Stable logical identifier for the item being planned.
    pub id: String,
    /// Relative weight used when balancing items across bins.
    pub weight: f64,
}

/// Supported planner strategies.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TopLevelPlannerMode {
    /// Deterministic round-robin assignment by item id.
    RoundRobin,
    /// Deterministic greedy balancing with no hysteresis or cooldown.
    Greedy,
    /// Greedy balancing that prefers to keep prior assignments unless a move is worthwhile.
    #[default]
    StableGreedy,
}

/// Tunables for top-level queue planning.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TopLevelPlannerParams {
    /// Planning mode.
    pub mode: TopLevelPlannerMode,
    /// Fraction of total weight improvement required before a move is considered worthwhile.
    pub hysteresis_threshold: f64,
    /// Minimum seconds between moves for a previously assigned item.
    pub cooldown_seconds: f64,
    /// Maximum number of assignment changes allowed in a single planning run.
    pub move_budget_per_run: usize,
}

impl Default for TopLevelPlannerParams {
    fn default() -> Self {
        Self {
            mode: TopLevelPlannerMode::StableGreedy,
            hysteresis_threshold: 0.03,
            cooldown_seconds: 3600.0,
            move_budget_per_run: 1,
        }
    }
}

/// Result of a top-level assignment pass.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TopLevelPlannerOutput {
    /// Final per-item bin assignment.
    pub assignment: BTreeMap<String, String>,
    /// Item ids whose assigned bin changed relative to the previous assignment map.
    pub changed: Vec<String>,
    /// Whether the stable planner path, rather than a simple fallback, was used.
    pub planner_used: bool,
}

/// Input describing a site that needs class identity assignment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SiteIdentityInput {
    /// Stable path key for the site within the logical hierarchy.
    pub site_key: String,
    /// Stable path key for the parent site, or empty for top-level nodes.
    pub parent_path: String,
    /// Target shaping queue number, 1-based.
    pub queue: u32,
    /// Whether this site has child sites in the physical tree.
    pub has_children: bool,
}

/// Assigned class identity for a site.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SiteIdentityAssignment {
    /// Stable path key for the site.
    pub site_key: String,
    /// Assigned queue number, 1-based.
    pub queue: u32,
    /// Assigned class minor.
    pub class_minor: u16,
    /// Assigned downlink class major.
    pub class_major: u16,
    /// Assigned uplink class major.
    pub up_class_major: u16,
    /// Parent path used when deciding minor reuse.
    pub parent_path: String,
}

/// Input describing a circuit group under a single parent site.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitIdentityGroupInput {
    /// Parent node name for this group of circuits.
    pub parent_node: String,
    /// Queue number, 1-based.
    pub queue: u32,
    /// Circuit ids in deterministic order for this parent.
    pub circuit_ids: Vec<String>,
}

/// Assigned class identity for a circuit.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitIdentityAssignment {
    /// Stable circuit id.
    pub circuit_id: String,
    /// Parent node used when deciding minor reuse.
    pub parent_node: String,
    /// Assigned queue number, 1-based.
    pub queue: u32,
    /// Assigned class minor.
    pub class_minor: u16,
    /// Assigned downlink class major.
    pub class_major: u16,
    /// Assigned uplink class major.
    pub up_class_major: u16,
}

/// Persisted planner identity for a site.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlannerSiteIdentityState {
    /// Assigned class minor.
    pub class_minor: u16,
    /// Assigned queue number, 1-based.
    pub queue: u32,
    /// Stable parent path for reuse checks.
    pub parent_path: String,
    /// Downlink class major.
    pub class_major: u16,
    /// Uplink class major.
    pub up_class_major: u16,
}

/// Persisted planner identity for a circuit.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlannerCircuitIdentityState {
    /// Assigned class minor.
    pub class_minor: u16,
    /// Assigned queue number, 1-based.
    pub queue: u32,
    /// Stable parent node for reuse checks.
    pub parent_node: String,
    /// Downlink class major.
    pub class_major: u16,
    /// Uplink class major.
    pub up_class_major: u16,
}

/// Output of site/circuit identity assignment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClassIdentityPlannerOutput {
    /// Assigned site identities keyed in traversal order.
    pub sites: Vec<SiteIdentityAssignment>,
    /// Assigned circuit identities keyed in traversal order.
    pub circuits: Vec<CircuitIdentityAssignment>,
    /// Planner-state site map suitable for persistence.
    pub site_state: BTreeMap<String, PlannerSiteIdentityState>,
    /// Planner-state circuit map suitable for persistence.
    pub circuit_state: BTreeMap<String, PlannerCircuitIdentityState>,
    /// Highest allocated minor counter per queue after assignment.
    pub last_used_minor_by_queue: BTreeMap<u32, u32>,
}

/// Reserved minor numbers keyed by shaping queue.
pub type PlannerMinorReservations = BTreeMap<u32, BTreeSet<u32>>;

/// Structural constraints and numbering offsets for class identity planning.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassIdentityPlannerConstraints {
    /// Reserved site minors keyed by queue.
    pub reserved_site_minors: PlannerMinorReservations,
    /// Reserved circuit minors keyed by queue.
    pub reserved_circuit_minors: PlannerMinorReservations,
    /// Starting minor for planned site classes.
    pub site_minor_start: u32,
    /// Starting minor for planned circuit classes.
    pub circuit_minor_start: u32,
    /// Uplink major offset for on-a-stick layouts.
    pub stick_offset: u16,
    /// Padding to leave after each circuit group allocation.
    pub circuit_padding: u32,
}

fn sanitize_weight(weight: f64) -> f64 {
    if !weight.is_finite() || weight <= 0.0 {
        1.0
    } else {
        weight
    }
}

fn next_free_minor(start_minor: u32, reserved: &BTreeSet<u32>) -> u32 {
    let mut candidate = start_minor.max(3);
    while reserved.contains(&candidate) {
        candidate += 1;
    }
    candidate
}

fn valid_class_minor(class_minor: u16) -> Option<u32> {
    let class_minor = u32::from(class_minor);
    (3..u32::from(u16::MAX))
        .contains(&class_minor)
        .then_some(class_minor)
}

fn expected_class_majors(queue: u32, stick_offset: u16) -> Option<(u16, u16)> {
    let class_major = u16::try_from(queue).ok()?;
    (class_major > 0).then_some((class_major, class_major.saturating_add(stick_offset)))
}

fn reserve_reusable_site_minors(
    sites: &[SiteIdentityInput],
    previous_sites: &BTreeMap<String, PlannerSiteIdentityState>,
    stick_offset: u16,
    reserved: &mut PlannerMinorReservations,
) -> BTreeMap<String, u32> {
    let current_sites = sites
        .iter()
        .map(|site| (site.site_key.as_str(), site))
        .collect::<BTreeMap<_, _>>();
    let mut reusable = BTreeMap::new();

    for (site_key, site) in current_sites {
        let Some(stored) = previous_sites.get(site_key) else {
            continue;
        };
        let Some(class_minor) = valid_class_minor(stored.class_minor) else {
            continue;
        };
        let Some((class_major, up_class_major)) = expected_class_majors(site.queue, stick_offset)
        else {
            continue;
        };
        if stored.queue != site.queue
            || stored.parent_path != site.parent_path
            || stored.class_major != class_major
            || stored.up_class_major != up_class_major
        {
            continue;
        }

        let occupied = reserved.entry(site.queue).or_default();
        if occupied.insert(class_minor) {
            reusable.insert(site_key.to_string(), class_minor);
        }
    }

    reusable
}

fn reserve_reusable_circuit_minors(
    circuit_groups: &[CircuitIdentityGroupInput],
    previous_circuits: &BTreeMap<String, PlannerCircuitIdentityState>,
    stick_offset: u16,
    reserved: &mut PlannerMinorReservations,
) -> BTreeMap<String, u32> {
    let current_circuits = circuit_groups
        .iter()
        .flat_map(|group| {
            group
                .circuit_ids
                .iter()
                .map(move |circuit_id| (circuit_id.as_str(), group))
        })
        .collect::<BTreeMap<_, _>>();
    let mut reusable = BTreeMap::new();

    for (circuit_id, group) in current_circuits {
        let Some(stored) = previous_circuits.get(circuit_id) else {
            continue;
        };
        let Some(class_minor) = valid_class_minor(stored.class_minor) else {
            continue;
        };
        let Some((class_major, up_class_major)) = expected_class_majors(group.queue, stick_offset)
        else {
            continue;
        };
        if stored.queue != group.queue
            || stored.parent_node != group.parent_node
            || stored.class_major != class_major
            || stored.up_class_major != up_class_major
        {
            continue;
        }

        let occupied = reserved.entry(group.queue).or_default();
        if occupied.insert(class_minor) {
            reusable.insert(circuit_id.to_string(), class_minor);
        }
    }

    reusable
}

/// Builds reservation maps for identities that are not part of the current planning scope.
pub fn build_class_identity_reservations(
    sites: &[SiteIdentityInput],
    circuit_groups: &[CircuitIdentityGroupInput],
    previous_sites: &BTreeMap<String, PlannerSiteIdentityState>,
    previous_circuits: &BTreeMap<String, PlannerCircuitIdentityState>,
) -> (PlannerMinorReservations, PlannerMinorReservations) {
    let planned_site_keys: BTreeSet<&str> =
        sites.iter().map(|site| site.site_key.as_str()).collect();
    let planned_circuit_ids: BTreeSet<&str> = circuit_groups
        .iter()
        .flat_map(|group| group.circuit_ids.iter().map(|id| id.as_str()))
        .collect();

    let mut reserved_site_minors: PlannerMinorReservations = BTreeMap::new();
    for (site_key, stored) in previous_sites {
        if planned_site_keys.contains(site_key.as_str()) {
            continue;
        }
        if let Some(class_minor) = valid_class_minor(stored.class_minor) {
            reserved_site_minors
                .entry(stored.queue)
                .or_default()
                .insert(class_minor);
        }
    }

    let mut reserved_circuit_minors: PlannerMinorReservations = BTreeMap::new();
    for (circuit_id, stored) in previous_circuits {
        if planned_circuit_ids.contains(circuit_id.as_str()) {
            continue;
        }
        if let Some(class_minor) = valid_class_minor(stored.class_minor) {
            reserved_circuit_minors
                .entry(stored.queue)
                .or_default()
                .insert(class_minor);
        }
    }

    (reserved_site_minors, reserved_circuit_minors)
}

fn round_robin_assign(items: &[TopLevelPlannerItem], bins: &[String]) -> BTreeMap<String, String> {
    let mut ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    ids.sort_unstable();

    let mut assignment = BTreeMap::new();
    for (idx, id) in ids.into_iter().enumerate() {
        if let Some(bin) = bins.get(idx % bins.len()) {
            assignment.insert(id.to_string(), bin.clone());
        }
    }
    assignment
}

fn greedy_assign(items: &[TopLevelPlannerItem], bins: &[String]) -> BTreeMap<String, String> {
    let mut pairs: Vec<(&str, f64)> = items
        .iter()
        .map(|item| (item.id.as_str(), sanitize_weight(item.weight)))
        .collect();
    pairs.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });

    let mut loads: BTreeMap<String, f64> = bins.iter().cloned().map(|bin| (bin, 0.0)).collect();
    let mut assignment = BTreeMap::new();
    for (id, weight) in pairs {
        let target = loads
            .iter()
            .min_by(|a, b| {
                a.1.partial_cmp(b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(b.0))
            })
            .map(|(bin, _)| bin.clone())
            .unwrap_or_else(|| bins[0].clone());
        assignment.insert(id.to_string(), target.clone());
        if let Some(load) = loads.get_mut(&target) {
            *load += weight;
        }
    }
    assignment
}

fn load_by_bin(
    assignment: &BTreeMap<String, String>,
    item_weights: &BTreeMap<String, f64>,
    bins: &[String],
) -> BTreeMap<String, f64> {
    let mut loads: BTreeMap<String, f64> = bins.iter().cloned().map(|bin| (bin, 0.0)).collect();
    for (id, bin) in assignment {
        if let Some(weight) = item_weights.get(id)
            && let Some(load) = loads.get_mut(bin)
        {
            *load += *weight;
        }
    }
    loads
}

/// Plans top-level queue assignments for the given logical items and bins.
///
/// This function is pure: it computes a deterministic assignment and does not perform any I/O.
pub fn plan_top_level_assignments(
    items: &[TopLevelPlannerItem],
    bins: &[String],
    prev_assign: &BTreeMap<String, String>,
    last_change_ts: &BTreeMap<String, f64>,
    now_ts: f64,
    params: &TopLevelPlannerParams,
) -> TopLevelPlannerOutput {
    if bins.is_empty() || items.is_empty() {
        return TopLevelPlannerOutput {
            assignment: BTreeMap::new(),
            changed: Vec::new(),
            planner_used: !matches!(params.mode, TopLevelPlannerMode::RoundRobin),
        };
    }

    let item_ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
    let item_weights: BTreeMap<String, f64> = items
        .iter()
        .map(|item| (item.id.clone(), sanitize_weight(item.weight)))
        .collect();
    let valid_bins: std::collections::BTreeSet<&String> = bins.iter().collect();
    let prev_valid: BTreeMap<String, String> = prev_assign
        .iter()
        .filter(|(id, bin)| item_weights.contains_key(*id) && valid_bins.contains(*bin))
        .map(|(id, bin)| (id.clone(), bin.clone()))
        .collect();

    let rr_assignment = round_robin_assign(items, bins);
    let greedy_assignment = greedy_assign(items, bins);

    let (assignment, planner_used) = match params.mode {
        TopLevelPlannerMode::RoundRobin => (rr_assignment.clone(), false),
        TopLevelPlannerMode::Greedy => (greedy_assignment.clone(), false),
        TopLevelPlannerMode::StableGreedy => {
            let mut assignment = rr_assignment.clone();
            for id in &item_ids {
                let chosen = prev_valid
                    .get(id)
                    .cloned()
                    .or_else(|| greedy_assignment.get(id).cloned())
                    .or_else(|| rr_assignment.get(id).cloned())
                    .unwrap_or_else(|| bins[0].clone());
                assignment.insert(id.clone(), chosen);
            }

            let mut loads = load_by_bin(&assignment, &item_weights, bins);
            let total_weight: f64 = item_weights.values().sum();
            let min_improvement = total_weight * params.hysteresis_threshold.max(0.0);
            let mut weighted_ids = item_ids.clone();
            weighted_ids.sort_by(|a, b| {
                item_weights[b]
                    .partial_cmp(&item_weights[a])
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.cmp(b))
            });

            let mut moves_used = 0usize;
            for id in &weighted_ids {
                let Some(current_bin) = assignment.get(id).cloned() else {
                    continue;
                };
                let Some(target_bin) = greedy_assignment.get(id).cloned() else {
                    continue;
                };
                if current_bin == target_bin {
                    continue;
                }
                if moves_used >= params.move_budget_per_run {
                    continue;
                }
                let last_changed = last_change_ts.get(id).copied().unwrap_or(0.0);
                if now_ts - last_changed < params.cooldown_seconds {
                    continue;
                }
                let current_load = loads.get(&current_bin).copied().unwrap_or(0.0);
                let target_load = loads.get(&target_bin).copied().unwrap_or(0.0);
                let weight = item_weights.get(id).copied().unwrap_or(1.0);
                let imbalance_before = current_load - target_load;
                let imbalance_after = ((current_load - weight) - (target_load + weight)).abs();
                if imbalance_before - imbalance_after <= min_improvement {
                    continue;
                }

                if let Some(load) = loads.get_mut(&current_bin) {
                    *load -= weight;
                }
                if let Some(load) = loads.get_mut(&target_bin) {
                    *load += weight;
                }
                assignment.insert(id.clone(), target_bin);
                moves_used += 1;
            }

            let used_bins = assignment.values().collect::<BTreeSet<_>>();
            if bins.len() > 1
                && assignment.len() > 1
                && used_bins.len() <= 1
                && moves_used < params.move_budget_per_run
            {
                for id in weighted_ids {
                    let Some(current_bin) = assignment.get(&id) else {
                        continue;
                    };
                    let Some(target_bin) = greedy_assignment.get(&id) else {
                        continue;
                    };
                    if current_bin != target_bin {
                        assignment.insert(id, target_bin.clone());
                        break;
                    }
                }
            }

            (assignment, true)
        }
    };

    let mut changed: Vec<String> = assignment
        .iter()
        .filter_map(|(id, bin)| {
            prev_valid
                .get(id)
                .and_then(|prev| (prev != bin).then_some(id.clone()))
        })
        .collect();
    changed.sort_unstable();

    TopLevelPlannerOutput {
        assignment,
        changed,
        planner_used,
    }
}

/// Plans site and circuit class identities for a physical queue tree.
///
/// This function is pure: it computes deterministic minor/class-major assignments from the
/// provided planner state and traversal order without performing any I/O.
pub fn plan_class_identities_with_constraints(
    sites: &[SiteIdentityInput],
    circuit_groups: &[CircuitIdentityGroupInput],
    previous_sites: &BTreeMap<String, PlannerSiteIdentityState>,
    previous_circuits: &BTreeMap<String, PlannerCircuitIdentityState>,
    constraints: &ClassIdentityPlannerConstraints,
) -> ClassIdentityPlannerOutput {
    let mut reserved_site_minors = constraints.reserved_site_minors.clone();
    for (queue, minors) in &constraints.reserved_circuit_minors {
        reserved_site_minors
            .entry(*queue)
            .or_default()
            .extend(minors.iter().copied());
    }
    let reusable_site_minors = reserve_reusable_site_minors(
        sites,
        previous_sites,
        constraints.stick_offset,
        &mut reserved_site_minors,
    );
    let reusable_circuit_minors = reserve_reusable_circuit_minors(
        circuit_groups,
        previous_circuits,
        constraints.stick_offset,
        &mut reserved_site_minors,
    );
    let mut next_site_minor_by_queue: BTreeMap<u32, u32> = BTreeMap::new();
    let mut site_assignments = Vec::with_capacity(sites.len());
    let mut site_state = BTreeMap::new();

    for site in sites {
        let reserved = reserved_site_minors.entry(site.queue).or_default();
        let next_minor = next_site_minor_by_queue
            .entry(site.queue)
            .or_insert(constraints.site_minor_start.max(3));
        let reuse_minor = reusable_site_minors.get(&site.site_key).copied();

        let class_minor_u32 = if let Some(reused_minor) = reuse_minor {
            reused_minor
        } else {
            let allocated_minor = next_free_minor(*next_minor, reserved);
            *next_minor = allocated_minor.saturating_add(1);
            if site.has_children {
                *next_minor = (*next_minor).saturating_add(1);
            }
            allocated_minor
        };
        let class_minor = u16::try_from(class_minor_u32).unwrap_or(u16::MAX);
        let class_major = u16::try_from(site.queue).unwrap_or(u16::MAX);
        let up_class_major = class_major.saturating_add(constraints.stick_offset);
        reserved.insert(class_minor_u32);

        let assigned = SiteIdentityAssignment {
            site_key: site.site_key.clone(),
            queue: site.queue,
            class_minor,
            class_major,
            up_class_major,
            parent_path: site.parent_path.clone(),
        };
        site_state.insert(
            site.site_key.clone(),
            PlannerSiteIdentityState {
                class_minor,
                queue: site.queue,
                parent_path: site.parent_path.clone(),
                class_major,
                up_class_major,
            },
        );
        site_assignments.push(assigned);
    }

    let mut reserved_circuit_minors = reserved_site_minors.clone();
    let mut next_circuit_minor_by_queue = BTreeMap::new();
    for (queue, reserved) in &reserved_circuit_minors {
        let start = next_site_minor_by_queue
            .get(queue)
            .copied()
            .unwrap_or_else(|| next_free_minor(constraints.circuit_minor_start.max(3), reserved));
        next_circuit_minor_by_queue.insert(
            *queue,
            next_free_minor(start.max(constraints.circuit_minor_start.max(3)), reserved),
        );
    }

    let mut circuit_assignments = Vec::new();
    let mut circuit_state = BTreeMap::new();
    for group in circuit_groups {
        let reserved = reserved_circuit_minors.entry(group.queue).or_default();
        let next_minor = next_circuit_minor_by_queue
            .entry(group.queue)
            .or_insert_with(|| next_free_minor(constraints.circuit_minor_start.max(3), reserved));
        for circuit_id in &group.circuit_ids {
            let reuse_minor = reusable_circuit_minors.get(circuit_id).copied();
            let class_minor_u32 = if let Some(reused_minor) = reuse_minor {
                reused_minor
            } else {
                let allocated_minor = next_free_minor(*next_minor, reserved);
                *next_minor = allocated_minor.saturating_add(1);
                allocated_minor
            };
            let class_minor = u16::try_from(class_minor_u32).unwrap_or(u16::MAX);
            let class_major = u16::try_from(group.queue).unwrap_or(u16::MAX);
            let up_class_major = class_major.saturating_add(constraints.stick_offset);
            reserved.insert(class_minor_u32);

            circuit_assignments.push(CircuitIdentityAssignment {
                circuit_id: circuit_id.clone(),
                parent_node: group.parent_node.clone(),
                queue: group.queue,
                class_minor,
                class_major,
                up_class_major,
            });
            circuit_state.insert(
                circuit_id.clone(),
                PlannerCircuitIdentityState {
                    class_minor,
                    queue: group.queue,
                    parent_node: group.parent_node.clone(),
                    class_major,
                    up_class_major,
                },
            );
        }
        *next_minor = (*next_minor).saturating_add(constraints.circuit_padding);
    }

    let mut last_used_minor_by_queue = BTreeMap::new();
    for queue in reserved_circuit_minors
        .keys()
        .chain(next_site_minor_by_queue.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<u32>>()
    {
        let site_next = next_site_minor_by_queue.get(&queue).copied().unwrap_or(3);
        let circuit_next = next_circuit_minor_by_queue
            .get(&queue)
            .copied()
            .unwrap_or(3);
        let highest_reserved_next = reserved_circuit_minors
            .get(&queue)
            .and_then(|reserved| reserved.iter().next_back().copied())
            .unwrap_or(2)
            .saturating_add(1);
        last_used_minor_by_queue.insert(
            queue,
            site_next.max(circuit_next).max(highest_reserved_next),
        );
    }

    ClassIdentityPlannerOutput {
        sites: site_assignments,
        circuits: circuit_assignments,
        site_state,
        circuit_state,
        last_used_minor_by_queue,
    }
}

/// Plans site and circuit class identities using the default structural allocator policy.
pub fn plan_class_identities(
    sites: &[SiteIdentityInput],
    circuit_groups: &[CircuitIdentityGroupInput],
    previous_sites: &BTreeMap<String, PlannerSiteIdentityState>,
    previous_circuits: &BTreeMap<String, PlannerCircuitIdentityState>,
    stick_offset: u16,
    circuit_padding: u32,
) -> ClassIdentityPlannerOutput {
    let (reserved_site_minors, reserved_circuit_minors) =
        build_class_identity_reservations(sites, circuit_groups, previous_sites, previous_circuits);
    let constraints = ClassIdentityPlannerConstraints {
        reserved_site_minors,
        reserved_circuit_minors,
        site_minor_start: 3,
        circuit_minor_start: 3,
        stick_offset,
        circuit_padding,
    };
    plan_class_identities_with_constraints(
        sites,
        circuit_groups,
        previous_sites,
        previous_circuits,
        &constraints,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CircuitIdentityGroupInput, ClassIdentityPlannerConstraints, PlannerCircuitIdentityState,
        PlannerSiteIdentityState, SiteIdentityInput, TopLevelPlannerItem, TopLevelPlannerMode,
        TopLevelPlannerParams, build_class_identity_reservations, plan_class_identities,
        plan_class_identities_with_constraints, plan_top_level_assignments,
    };
    use std::collections::BTreeMap;

    fn bins() -> Vec<String> {
        vec!["CpueQueue0".to_string(), "CpueQueue1".to_string()]
    }

    #[test]
    fn round_robin_is_deterministic() {
        let items = vec![
            TopLevelPlannerItem {
                id: "b".to_string(),
                weight: 1.0,
            },
            TopLevelPlannerItem {
                id: "a".to_string(),
                weight: 1.0,
            },
        ];
        let result = plan_top_level_assignments(
            &items,
            &bins(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            0.0,
            &TopLevelPlannerParams {
                mode: TopLevelPlannerMode::RoundRobin,
                ..Default::default()
            },
        );
        assert_eq!(
            result.assignment.get("a").map(String::as_str),
            Some("CpueQueue0")
        );
        assert_eq!(
            result.assignment.get("b").map(String::as_str),
            Some("CpueQueue1")
        );
        assert!(!result.planner_used);
    }

    #[test]
    fn stable_greedy_breaks_degenerate_single_bin_assignments() {
        let items = vec![
            TopLevelPlannerItem {
                id: "heavy".to_string(),
                weight: 100.0,
            },
            TopLevelPlannerItem {
                id: "light".to_string(),
                weight: 1.0,
            },
        ];
        let prev_assign = BTreeMap::from([
            ("heavy".to_string(), "CpueQueue0".to_string()),
            ("light".to_string(), "CpueQueue0".to_string()),
        ]);
        let last_change_ts =
            BTreeMap::from([("heavy".to_string(), 990.0), ("light".to_string(), 990.0)]);
        let result = plan_top_level_assignments(
            &items,
            &bins(),
            &prev_assign,
            &last_change_ts,
            1000.0,
            &TopLevelPlannerParams {
                mode: TopLevelPlannerMode::StableGreedy,
                cooldown_seconds: 3600.0,
                move_budget_per_run: 1,
                hysteresis_threshold: 0.0,
            },
        );
        assert_eq!(
            result.assignment.get("light").map(String::as_str),
            Some("CpueQueue1")
        );
        assert_eq!(result.changed, vec!["light".to_string()]);
    }

    #[test]
    fn stable_greedy_can_move_when_cooldown_expired() {
        let items = vec![
            TopLevelPlannerItem {
                id: "heavy".to_string(),
                weight: 100.0,
            },
            TopLevelPlannerItem {
                id: "light".to_string(),
                weight: 1.0,
            },
        ];
        let prev_assign = BTreeMap::from([
            ("heavy".to_string(), "CpueQueue0".to_string()),
            ("light".to_string(), "CpueQueue0".to_string()),
        ]);
        let result = plan_top_level_assignments(
            &items,
            &bins(),
            &prev_assign,
            &BTreeMap::new(),
            10_000.0,
            &TopLevelPlannerParams {
                mode: TopLevelPlannerMode::StableGreedy,
                cooldown_seconds: 0.0,
                move_budget_per_run: 1,
                hysteresis_threshold: 0.0,
            },
        );
        assert_eq!(
            result.assignment.get("light").map(String::as_str),
            Some("CpueQueue1")
        );
        assert_eq!(result.changed, vec!["light".to_string()]);
        assert!(result.planner_used);
    }

    #[test]
    fn stable_greedy_does_not_make_balance_worse() {
        let items = vec![
            TopLevelPlannerItem {
                id: "a".to_string(),
                weight: 2.0,
            },
            TopLevelPlannerItem {
                id: "b".to_string(),
                weight: 2.0,
            },
            TopLevelPlannerItem {
                id: "c".to_string(),
                weight: 3.0,
            },
            TopLevelPlannerItem {
                id: "d".to_string(),
                weight: 5.0,
            },
            TopLevelPlannerItem {
                id: "e".to_string(),
                weight: 3.0,
            },
        ];
        let prev_assign = BTreeMap::from([
            ("a".to_string(), "CpueQueue1".to_string()),
            ("b".to_string(), "CpueQueue0".to_string()),
            ("c".to_string(), "CpueQueue1".to_string()),
            ("d".to_string(), "CpueQueue0".to_string()),
        ]);

        let result = plan_top_level_assignments(
            &items,
            &bins(),
            &prev_assign,
            &BTreeMap::new(),
            10_000.0,
            &TopLevelPlannerParams {
                mode: TopLevelPlannerMode::StableGreedy,
                cooldown_seconds: 0.0,
                move_budget_per_run: 1,
                hysteresis_threshold: 0.03,
            },
        );

        assert!(result.changed.is_empty());
        for (circuit_id, previous_bin) in prev_assign {
            assert_eq!(result.assignment[&circuit_id], previous_bin);
        }
    }

    #[test]
    fn stable_greedy_single_bin_recovery_respects_move_budget() {
        let items = (0..6)
            .map(|index| TopLevelPlannerItem {
                id: format!("circuit-{index}"),
                weight: 1.0,
            })
            .collect::<Vec<_>>();
        let prev_assign = items
            .iter()
            .map(|item| (item.id.clone(), "CpueQueue0".to_string()))
            .collect::<BTreeMap<_, _>>();
        let last_change_ts = items
            .iter()
            .map(|item| (item.id.clone(), 999.0))
            .collect::<BTreeMap<_, _>>();

        let result = plan_top_level_assignments(
            &items,
            &bins(),
            &prev_assign,
            &last_change_ts,
            1_000.0,
            &TopLevelPlannerParams {
                mode: TopLevelPlannerMode::StableGreedy,
                cooldown_seconds: 3_600.0,
                move_budget_per_run: 1,
                hysteresis_threshold: 0.0,
            },
        );

        assert_eq!(result.changed.len(), 1);
        assert_eq!(
            result
                .assignment
                .values()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            2
        );
    }

    #[test]
    fn class_identity_planner_reuses_site_and_circuit_minors() {
        let site_inputs = vec![
            SiteIdentityInput {
                site_key: "CpueQueue0/site-a".to_string(),
                parent_path: "CpueQueue0".to_string(),
                queue: 1,
                has_children: true,
            },
            SiteIdentityInput {
                site_key: "CpueQueue0/site-a/child".to_string(),
                parent_path: "CpueQueue0/site-a".to_string(),
                queue: 1,
                has_children: false,
            },
        ];
        let circuit_groups = vec![CircuitIdentityGroupInput {
            parent_node: "child".to_string(),
            queue: 1,
            circuit_ids: vec!["c1".to_string(), "c2".to_string()],
        }];
        let prev_sites = BTreeMap::from([
            (
                "CpueQueue0/site-a".to_string(),
                PlannerSiteIdentityState {
                    class_minor: 0x20,
                    queue: 1,
                    parent_path: "CpueQueue0".to_string(),
                    class_major: 1,
                    up_class_major: 0x41,
                },
            ),
            (
                "CpueQueue0/site-a/child".to_string(),
                PlannerSiteIdentityState {
                    class_minor: 0x22,
                    queue: 1,
                    parent_path: "CpueQueue0/site-a".to_string(),
                    class_major: 1,
                    up_class_major: 0x41,
                },
            ),
        ]);
        let prev_circuits = BTreeMap::from([(
            "c1".to_string(),
            PlannerCircuitIdentityState {
                class_minor: 0x30,
                queue: 1,
                parent_node: "child".to_string(),
                class_major: 1,
                up_class_major: 0x41,
            },
        )]);

        let result = plan_class_identities(
            &site_inputs,
            &circuit_groups,
            &prev_sites,
            &prev_circuits,
            0x40,
            8,
        );

        assert_eq!(result.sites[0].class_minor, 0x20);
        assert_eq!(result.sites[1].class_minor, 0x22);
        assert_eq!(result.circuits[0].class_minor, 0x30);
        assert_eq!(result.circuits[1].queue, 1);
        assert_eq!(result.circuits[1].class_major, 1);
    }

    #[test]
    fn class_identity_planner_reserves_untouched_previous_minors() {
        let site_inputs = vec![SiteIdentityInput {
            site_key: "CpueQueue6/site-moved".to_string(),
            parent_path: "CpueQueue6/site-parent".to_string(),
            queue: 7,
            has_children: false,
        }];
        let prev_sites = BTreeMap::from([
            (
                "CpueQueue6/site-untouched".to_string(),
                PlannerSiteIdentityState {
                    class_minor: 0x29,
                    queue: 7,
                    parent_path: "".to_string(),
                    class_major: 7,
                    up_class_major: 7,
                },
            ),
            (
                "CpueQueue0/site-moved".to_string(),
                PlannerSiteIdentityState {
                    class_minor: 0x13,
                    queue: 1,
                    parent_path: "CpueQueue0/site-parent".to_string(),
                    class_major: 1,
                    up_class_major: 1,
                },
            ),
        ]);

        let result = plan_class_identities(&site_inputs, &[], &prev_sites, &BTreeMap::new(), 0, 0);

        assert_eq!(result.sites.len(), 1);
        assert_eq!(result.sites[0].queue, 7);
        assert_ne!(result.sites[0].class_minor, 0x29);
    }

    #[test]
    fn class_identity_planner_compacts_when_full_reload_ignores_saved_minors() {
        const FRAGMENTED_SITE_MINOR: u16 = 0xff00;
        const FRAGMENTED_CHILD_MINOR: u16 = 0xff02;
        const FRAGMENTED_CIRCUIT_MINOR: u16 = 0xff10;
        const CIRCUIT_PADDING: u32 = 8;

        let site_inputs = vec![
            SiteIdentityInput {
                site_key: "CpueQueue0/site-a".to_string(),
                parent_path: "CpueQueue0".to_string(),
                queue: 1,
                has_children: true,
            },
            SiteIdentityInput {
                site_key: "CpueQueue0/site-a/child".to_string(),
                parent_path: "CpueQueue0/site-a".to_string(),
                queue: 1,
                has_children: false,
            },
        ];
        let circuit_groups = vec![CircuitIdentityGroupInput {
            parent_node: "child".to_string(),
            queue: 1,
            circuit_ids: vec!["c1".to_string(), "c2".to_string()],
        }];
        let fragmented_sites = BTreeMap::from([
            (
                "CpueQueue0/site-a".to_string(),
                PlannerSiteIdentityState {
                    class_minor: FRAGMENTED_SITE_MINOR,
                    queue: 1,
                    parent_path: "CpueQueue0".to_string(),
                    class_major: 1,
                    up_class_major: 0x41,
                },
            ),
            (
                "CpueQueue0/site-a/child".to_string(),
                PlannerSiteIdentityState {
                    class_minor: FRAGMENTED_CHILD_MINOR,
                    queue: 1,
                    parent_path: "CpueQueue0/site-a".to_string(),
                    class_major: 1,
                    up_class_major: 0x41,
                },
            ),
        ]);
        let fragmented_circuits = BTreeMap::from([(
            "c1".to_string(),
            PlannerCircuitIdentityState {
                class_minor: FRAGMENTED_CIRCUIT_MINOR,
                queue: 1,
                parent_node: "child".to_string(),
                class_major: 1,
                up_class_major: 0x41,
            },
        )]);

        let sticky_result = plan_class_identities(
            &site_inputs,
            &circuit_groups,
            &fragmented_sites,
            &fragmented_circuits,
            0x40,
            CIRCUIT_PADDING,
        );
        assert!(sticky_result.last_used_minor_by_queue[&1] > u32::from(FRAGMENTED_CIRCUIT_MINOR));

        let full_reload_result = plan_class_identities(
            &site_inputs,
            &circuit_groups,
            &BTreeMap::new(),
            &BTreeMap::new(),
            0x40,
            CIRCUIT_PADDING,
        );

        let expected_last_minor = u32::from(full_reload_result.circuits[1].class_minor)
            .saturating_add(1)
            .saturating_add(CIRCUIT_PADDING);
        assert_eq!(full_reload_result.sites[0].class_minor, 3);
        assert_eq!(full_reload_result.sites[1].class_minor, 5);
        assert_eq!(full_reload_result.circuits[0].class_minor, 6);
        assert_eq!(full_reload_result.circuits[1].class_minor, 7);
        assert_eq!(
            full_reload_result.last_used_minor_by_queue[&1],
            expected_last_minor
        );
    }

    #[test]
    fn class_identity_planner_runtime_constraints_prefer_runtime_band() {
        let site_inputs = vec![SiteIdentityInput {
            site_key: "CpueQueue6/site-runtime".to_string(),
            parent_path: "CpueQueue6/site-parent".to_string(),
            queue: 7,
            has_children: false,
        }];
        let prev_sites = BTreeMap::from([(
            "CpueQueue6/site-untouched".to_string(),
            PlannerSiteIdentityState {
                class_minor: 0x29,
                queue: 7,
                parent_path: "".to_string(),
                class_major: 7,
                up_class_major: 7,
            },
        )]);
        let (reserved_site_minors, reserved_circuit_minors) =
            build_class_identity_reservations(&site_inputs, &[], &prev_sites, &BTreeMap::new());
        let constraints = ClassIdentityPlannerConstraints {
            reserved_site_minors,
            reserved_circuit_minors,
            site_minor_start: 0x1000,
            circuit_minor_start: 0x1000,
            stick_offset: 0,
            circuit_padding: 0,
        };

        let result = plan_class_identities_with_constraints(
            &site_inputs,
            &[],
            &BTreeMap::new(),
            &BTreeMap::new(),
            &constraints,
        );

        assert_eq!(result.sites.len(), 1);
        assert!(result.sites[0].class_minor >= 0x1000);
    }

    #[test]
    fn class_identity_planner_protects_existing_circuits_from_sorted_insertions() {
        let circuit_groups = vec![CircuitIdentityGroupInput {
            parent_node: "flat-bucket".to_string(),
            queue: 1,
            circuit_ids: vec![
                "a-new".to_string(),
                "m-existing".to_string(),
                "z-existing".to_string(),
            ],
        }];
        let previous_circuits = BTreeMap::from([
            (
                "m-existing".to_string(),
                PlannerCircuitIdentityState {
                    class_minor: 10,
                    queue: 1,
                    parent_node: "flat-bucket".to_string(),
                    class_major: 1,
                    up_class_major: 1,
                },
            ),
            (
                "z-existing".to_string(),
                PlannerCircuitIdentityState {
                    class_minor: 11,
                    queue: 1,
                    parent_node: "flat-bucket".to_string(),
                    class_major: 1,
                    up_class_major: 1,
                },
            ),
        ]);

        let result = plan_class_identities(
            &[],
            &circuit_groups,
            &BTreeMap::new(),
            &previous_circuits,
            0,
            0,
        );

        assert_eq!(result.circuit_state["m-existing"].class_minor, 10);
        assert_eq!(result.circuit_state["z-existing"].class_minor, 11);
        assert_eq!(result.circuit_state["a-new"].class_minor, 3);

        let repeated = plan_class_identities(
            &[],
            &circuit_groups,
            &BTreeMap::new(),
            &result.circuit_state,
            0,
            0,
        );
        assert_eq!(repeated.circuit_state, result.circuit_state);
        assert_eq!(
            repeated.last_used_minor_by_queue,
            result.last_used_minor_by_queue
        );
    }

    #[test]
    fn class_identity_planner_reassigns_only_the_circuit_that_moved() {
        let circuit_groups = vec![
            CircuitIdentityGroupInput {
                parent_node: "bucket-a".to_string(),
                queue: 1,
                circuit_ids: vec!["stable".to_string()],
            },
            CircuitIdentityGroupInput {
                parent_node: "bucket-b".to_string(),
                queue: 2,
                circuit_ids: vec!["moved".to_string()],
            },
        ];
        let previous_circuits = BTreeMap::from([
            (
                "stable".to_string(),
                PlannerCircuitIdentityState {
                    class_minor: 20,
                    queue: 1,
                    parent_node: "bucket-a".to_string(),
                    class_major: 1,
                    up_class_major: 1,
                },
            ),
            (
                "moved".to_string(),
                PlannerCircuitIdentityState {
                    class_minor: 21,
                    queue: 1,
                    parent_node: "bucket-a".to_string(),
                    class_major: 1,
                    up_class_major: 1,
                },
            ),
        ]);

        let result = plan_class_identities(
            &[],
            &circuit_groups,
            &BTreeMap::new(),
            &previous_circuits,
            0,
            0,
        );

        assert_eq!(result.circuit_state["stable"], previous_circuits["stable"]);
        assert_ne!(result.circuit_state["moved"], previous_circuits["moved"]);
        assert_eq!(result.circuit_state["moved"].queue, 2);
        assert_eq!(result.circuit_state["moved"].parent_node, "bucket-b");
    }

    #[test]
    fn class_identity_planner_rejects_duplicate_stale_and_invalid_saved_identities() {
        let circuit_groups = vec![CircuitIdentityGroupInput {
            parent_node: "flat-bucket".to_string(),
            queue: 1,
            circuit_ids: vec![
                "duplicate-a".to_string(),
                "duplicate-b".to_string(),
                "stale-parent".to_string(),
                "reserved-minor".to_string(),
            ],
        }];
        let previous_circuits = BTreeMap::from([
            (
                "duplicate-a".to_string(),
                PlannerCircuitIdentityState {
                    class_minor: 10,
                    queue: 1,
                    parent_node: "flat-bucket".to_string(),
                    class_major: 1,
                    up_class_major: 1,
                },
            ),
            (
                "duplicate-b".to_string(),
                PlannerCircuitIdentityState {
                    class_minor: 10,
                    queue: 1,
                    parent_node: "flat-bucket".to_string(),
                    class_major: 1,
                    up_class_major: 1,
                },
            ),
            (
                "stale-parent".to_string(),
                PlannerCircuitIdentityState {
                    class_minor: 11,
                    queue: 1,
                    parent_node: "old-bucket".to_string(),
                    class_major: 1,
                    up_class_major: 1,
                },
            ),
            (
                "reserved-minor".to_string(),
                PlannerCircuitIdentityState {
                    class_minor: u16::MAX,
                    queue: 1,
                    parent_node: "flat-bucket".to_string(),
                    class_major: 1,
                    up_class_major: 1,
                },
            ),
        ]);

        let result = plan_class_identities(
            &[],
            &circuit_groups,
            &BTreeMap::new(),
            &previous_circuits,
            0,
            0,
        );
        let assigned_minors = result
            .circuits
            .iter()
            .map(|circuit| circuit.class_minor)
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(result.circuits.len(), assigned_minors.len());
        assert_eq!(result.circuit_state["duplicate-a"].class_minor, 10);
        assert_ne!(result.circuit_state["duplicate-b"].class_minor, 10);
        assert_ne!(result.circuit_state["stale-parent"].class_minor, 11);
        assert_ne!(result.circuit_state["reserved-minor"].class_minor, u16::MAX);
        assert!(
            assigned_minors
                .iter()
                .all(|minor| (3..u16::MAX).contains(minor))
        );
    }

    #[test]
    fn class_identity_planner_fills_holes_below_high_saved_minors() {
        let circuit_groups = vec![CircuitIdentityGroupInput {
            parent_node: "flat-bucket".to_string(),
            queue: 1,
            circuit_ids: vec!["existing".to_string(), "new".to_string()],
        }];
        let previous_circuits = BTreeMap::from([(
            "existing".to_string(),
            PlannerCircuitIdentityState {
                class_minor: u16::MAX - 1,
                queue: 1,
                parent_node: "flat-bucket".to_string(),
                class_major: 1,
                up_class_major: 1,
            },
        )]);

        let result = plan_class_identities(
            &[],
            &circuit_groups,
            &BTreeMap::new(),
            &previous_circuits,
            0,
            0,
        );

        assert_eq!(result.circuit_state["existing"].class_minor, u16::MAX - 1);
        assert_eq!(result.circuit_state["new"].class_minor, 3);
    }
}
