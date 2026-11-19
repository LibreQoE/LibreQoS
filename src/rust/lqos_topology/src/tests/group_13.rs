// u7 (second Oracle consult): `ip_ranges.ignore_subnets` is subtracted from a
// fallback's mapping prefixes — and from every ordinary device mapping — because
// explicit management exclusions outrank catch-all coverage. The helper is pure
// prefix arithmetic; the fold-level tests prove one circuit per surviving rule,
// the carried prefix list, and the loud defers.

use super::{Prefix, subtract_ignored_prefixes};
use std::net::IpAddr;

/// `prefixes` as the CIDR strings an operator reads.
fn cidrs(prefixes: &[Prefix]) -> Vec<String> {
    prefixes
        .iter()
        .map(|(address, prefix_len)| format!("{address}/{prefix_len}"))
        .collect()
}

/// One test CIDR as the helper's currency.
fn cidr(value: &str) -> Prefix {
    let (address, prefix_len) = value.split_once('/').expect("a test cidr carries a length");
    (
        address.parse::<IpAddr>().expect("the test address parses"),
        prefix_len.parse().expect("the test length parses"),
    )
}

/// The complement of `192.168.0.0/16` inside `0.0.0.0/0`, hand-derived from
/// prefix arithmetic (sibling per bisection level), not produced by the helper.
const IPV4_MINUS_192_168: [&str; 16] = [
    "0.0.0.0/1",
    "128.0.0.0/2",
    "192.0.0.0/9",
    "192.128.0.0/11",
    "192.160.0.0/13",
    "192.169.0.0/16",
    "192.170.0.0/15",
    "192.172.0.0/14",
    "192.176.0.0/12",
    "192.192.0.0/10",
    "193.0.0.0/8",
    "194.0.0.0/7",
    "196.0.0.0/6",
    "200.0.0.0/5",
    "208.0.0.0/4",
    "224.0.0.0/3",
];

/// A rule that lies entirely inside an ignore has an EMPTY complement — at the
/// rule's own mask, inside a broader ignore, and at both host boundaries.
#[test]
fn ignore_subtraction_drops_a_fully_ignored_rule() {
    assert_eq!(
        subtract_ignored_prefixes(cidr("10.60.0.0/24"), &[cidr("10.60.0.0/24")]),
        Some(Vec::new()),
        "a rule equal to the ignore is fully excluded"
    );
    assert_eq!(
        subtract_ignored_prefixes(cidr("10.60.0.0/24"), &[cidr("10.0.0.0/8")]),
        Some(Vec::new()),
        "a narrower rule inside a broader ignore is fully excluded"
    );
    assert_eq!(
        subtract_ignored_prefixes(cidr("10.60.0.7/32"), &[cidr("10.60.0.0/24")]),
        Some(Vec::new()),
        "the /32 boundary is contained like any other prefix"
    );
    assert_eq!(
        subtract_ignored_prefixes(cidr("2001:db8::7/128"), &[cidr("2001:db8::/32")]),
        Some(Vec::new()),
        "the /128 boundary is contained like any other prefix"
    );
}

/// An ignore INSIDE the rule is cut out by bisection, including the case where
/// the ignore shares the rule's network address (the first child of every
/// bisection, so the "contained" arm must fire rather than the split).
#[test]
fn ignore_subtraction_bisects_an_ignore_sharing_the_rule_network_address() {
    assert_eq!(
        cidrs(
            &subtract_ignored_prefixes(cidr("10.60.0.0/16"), &[cidr("10.60.0.0/24")])
                .expect("the rule is representable")
        ),
        vec![
            "10.60.1.0/24",
            "10.60.2.0/23",
            "10.60.4.0/22",
            "10.60.8.0/21",
            "10.60.16.0/20",
            "10.60.32.0/19",
            "10.60.64.0/18",
            "10.60.128.0/17",
        ],
        "the /16 keeps every fragment outside the ignored /24"
    );
}

/// Disjoint and cross-family ignores change nothing, and a prefix length its
/// family cannot hold is refused instead of being truncated into coverage.
#[test]
fn ignore_subtraction_keeps_a_rule_clear_of_the_ignores() {
    assert_eq!(
        subtract_ignored_prefixes(cidr("203.0.113.0/24"), &[cidr("192.168.0.0/16")]),
        Some(vec![cidr("203.0.113.0/24")]),
        "a disjoint ignore leaves the rule whole"
    );
    assert_eq!(
        subtract_ignored_prefixes(cidr("0.0.0.0/0"), &[cidr("::/0")]),
        Some(vec![cidr("0.0.0.0/0")]),
        "a v6 ignore never intersects a v4 rule"
    );
    assert_eq!(
        subtract_ignored_prefixes(cidr("::/0"), &[cidr("0.0.0.0/0")]),
        Some(vec![cidr("::/0")]),
        "a v4 ignore never intersects a v6 rule"
    );
    assert_eq!(
        subtract_ignored_prefixes((IpAddr::from([10, 0, 0, 0]), 33), &[]),
        None,
        "a length above the family width is refused, never truncated"
    );
}

/// Duplicate, nested and overlapping ignores cannot change the complement, and
/// the /0 case is exactly the hand-derived sibling list.
#[test]
fn ignore_subtraction_is_unchanged_by_duplicate_nested_and_overlapping_ignores() {
    let single = subtract_ignored_prefixes(cidr("0.0.0.0/0"), &[cidr("192.168.0.0/16")])
        .expect("the rule is representable");
    assert_eq!(cidrs(&single), IPV4_MINUS_192_168);
    let redundant = subtract_ignored_prefixes(
        cidr("0.0.0.0/0"),
        &[
            cidr("192.168.0.0/16"),
            cidr("192.168.0.0/16"),
            cidr("192.168.0.0/24"),
            cidr("192.168.128.0/18"),
        ],
    )
    .expect("the rule is representable");
    assert_eq!(
        redundant, single,
        "a duplicate, plus ignores nested inside the first, cannot change the complement"
    );
}

/// A2: the fallback is ONE circuit with ONE device carrying the whole complement
/// at the rule's rates and parent, and the row keeps the origin marker.
#[test]
fn catch_all_fallback_carries_the_permitted_prefixes_in_one_device() {
    let mut config = range_test_config(
        "u7-range-subtracted",
        vec![range_rule("dr-pool", "0.0.0.0/0", "site-1")],
    );
    config.ip_ranges.ignore_subnets = vec!["192.168.0.0/16".to_string()];
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");

    let fallback = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:dr-pool")
        .expect("a catch-all rule with permitted fragments must fold");
    assert_eq!(fallback.devices.len(), 1, "ONE device carries the complement");
    assert_eq!(fallback.devices[0].ipv4, IPV4_MINUS_192_168);
    assert_eq!(
        fallback.devices[0].ipv6,
        Vec::<String>::new(),
        "a v4 rule never lands in the v6 map list"
    );
    assert_eq!(
        (
            fallback.download_min_mbps,
            fallback.upload_min_mbps,
            fallback.download_max_mbps,
            fallback.upload_max_mbps
        ),
        (10.0, 5.0, 100.0, 50.0),
        "the computed prefix list does not change the rule's plan"
    );
    assert_eq!(fallback.effective_parent_node_name, "site-1");
    assert!(
        fallback.devices[0].dynamic,
        "the origin marker still names the fallback row"
    );
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic_range_ignored_subnet")),
        "a rule with a non-empty complement is not a refusal: {:?}",
        inputs.warnings
    );
}

/// u8 A1: an ignore entry that does not parse is SKIPPED, so the entries that
/// DID parse still prune and still subtract. A list that PARTLY parses never
/// defers the fallbacks either (the pre-u8 gate deferred every rule on one bad
/// line), and the parser never panics the publish thread (pre-u8 indexed
/// `split('/')[1]`).
#[test]
fn a_malformed_ignore_entry_is_skipped_and_the_valid_ignores_still_prune() {
    let mut config = range_test_config(
        "u8-range-lenient-ignore",
        vec![range_rule("dr-pool", "0.0.0.0/0", "site-1")],
    );
    // Malformed FIRST: a parser that stops at the first bad line (or panics on
    // it) must not take the valid entries with it.
    config.ip_ranges.ignore_subnets = vec![
        "not-a-subnet".to_string(),
        "192.168.0.0/16".to_string(),
    ];
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("a malformed ignore must not fail the publish")
        .expect("shaping inputs should be present");

    let fallback = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:dr-pool")
        .expect("the fallback still folds: one bad line does not disable the prune");
    assert_eq!(
        fallback.devices[0].ipv4, IPV4_MINUS_192_168,
        "the VALID ignore is subtracted; the malformed entry contributes nothing"
    );
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic_range_ignored_subnets_unusable")),
        "a list with one usable prefix subtracts and does not defer: {:?}",
        inputs.warnings
    );

    // The compiled world maps the same permitted coverage the fold produced.
    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    let mapped: Vec<String> = tx
        .mapping_ops
        .iter()
        .filter_map(|op| match op {
            lqos_dataplane_types::DataplaneMappingOp::Map {
                circuit_id,
                ip,
                prefix_len,
                ..
            } if circuit_id == "dynamic-range:dr-pool" => Some(format!("{ip}/{prefix_len}")),
            _ => None,
        })
        .collect();
    assert_eq!(mapped, IPV4_MINUS_192_168, "the catch-all maps its complement");
}

/// u8 REV-C1: a configured ignore list that yields NO usable prefix must DEFER
/// fallback synthesis. The lenient skip is right for one bad line among good
/// ones; with nothing parsed the subtraction is a no-op and a catch-all would
/// publish `0.0.0.0/0` — unfiltered coverage over exactly the subnets the
/// operator asked to exclude (the u7 Oracle ruling: invalid ignore
/// configuration defers loudly and never publishes an unfiltered /0).
#[test]
fn a_configured_ignore_list_with_no_usable_entry_defers_the_fallback() {
    let mut config = range_test_config(
        "u8-range-unusable-ignore",
        vec![
            range_rule("dr-pool", "0.0.0.0/0", "site-1"),
            range_rule("dr-narrow", "192.168.66.0/24", "site-1"),
        ],
    );
    // The mask-less entry class that used to PANIC, plus an unparsable address:
    // both are configured lines and neither yields coverage.
    config.ip_ranges.ignore_subnets = vec![
        "10.0.0.7".to_string(),
        "not-a-subnet".to_string(),
    ];
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("an unusable ignore list must not fail the publish")
        .expect("shaping inputs should be present");

    for circuit_id in ["dynamic-range:dr-pool", "dynamic-range:dr-narrow"] {
        assert!(
            !inputs
                .circuits
                .iter()
                .any(|circuit| circuit.circuit_id == circuit_id),
            "{circuit_id} must have NO fallback row: an unreadable exclusion is not \
             'no exclusions configured'"
        );
    }
    assert_eq!(
        inputs
            .warnings
            .iter()
            .filter(|warning| warning.contains("dynamic_range_ignored_subnets_unusable"))
            .count(),
        2,
        "every rule is named once: {:?}",
        inputs.warnings
    );
    assert!(
        inputs.warnings.iter().any(|warning| warning
            .contains("dynamic_range_ignored_subnets_unusable")
            && warning.contains("dr-pool")
            && warning.contains("no prefix Map")),
        "the deferral names the rule and the consequence: {:?}",
        inputs.warnings
    );

    // The compiled world carries no fallback Map either: the deferral is not a
    // row that was dropped later in the pipeline.
    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        !tx.mapping_ops.iter().any(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneMappingOp::Map { circuit_id, .. }
                if circuit_id.starts_with("dynamic-range:")
        )),
        "no fallback prefix Map reaches the data plane: {:?}",
        tx.mapping_ops
    );
}

/// The REV-C1 deferral is NARROW: an EMPTY configured list means "no ignores",
/// so the catch-all keeps its `/0` whole (the default install has no such rule,
/// but a rule with nothing configured to subtract is intended coverage, not a
/// config error). Without this probe a later reader could generalize the
/// deferral into "empty list defers" and silently disable every catch-all.
#[test]
fn an_unconfigured_ignore_list_still_publishes_the_catch_all() {
    let mut config = range_test_config(
        "u8-range-no-ignores",
        vec![range_rule("dr-pool", "0.0.0.0/0", "site-1")],
    );
    config.ip_ranges.ignore_subnets = Vec::new();
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("an empty ignore list must not fail the publish")
        .expect("shaping inputs should be present");

    let fallback = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:dr-pool")
        .expect("a catch-all with nothing configured to exclude still folds");
    assert_eq!(
        fallback.devices[0].ipv4,
        vec!["0.0.0.0/0".to_string()],
        "nothing configured means nothing subtracted"
    );
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic_range_ignored_subnets_unusable")),
        "an unconfigured list is not a config error: {:?}",
        inputs.warnings
    );
}

/// A3: an ordinary device mapping is a PREFIX. A broader mapping keeps the
/// fragments outside the ignores, a fully ignored mapping disappears, a mapping
/// clear of the ignores is untouched, and a device left with no permitted prefix
/// is dropped like any other all-ignored device.
#[test]
fn a_device_prefix_partly_inside_an_ignore_keeps_its_permitted_fragments() {
    let mut config = tunnel_test_config("u7-device-prefix-prune");
    config.ip_ranges.ignore_subnets = vec!["10.1.0.0/16".to_string(), "fd00::/8".to_string()];
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    fs::write(
        ConfigShapedDevices::path_for_config(&config),
        concat!(
            "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,",
            "Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,",
            "Download Max Mbps,Upload Max Mbps,Comment\n",
            "\"partial-1\",\"Partial\",\"partial-dev\",\"Partial\",\"site-1\",\"\",\"\",",
            "\"aa:bb:cc:dd:ee:01\",\"10.0.0.0/8,10.1.4.0/24,10.200.0.0/16\",\"fc00::/7\",",
            "10,5,100,50,\"\"\n",
            "\"ignored-1\",\"Ignored\",\"ignored-dev\",\"Ignored\",\"site-1\",\"\",\"\",",
            "\"aa:bb:cc:dd:ee:02\",\"10.1.0.0/16\",\"\",10,5,100,50,\"\"\n",
        ),
    )
    .expect("the provisioned csv should write");

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let partial = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "partial-1")
        .expect("a device with permitted prefixes must survive");
    assert_eq!(
        partial.devices[0].ipv4,
        vec![
            "10.0.0.0/16",
            "10.2.0.0/15",
            "10.4.0.0/14",
            "10.8.0.0/13",
            "10.16.0.0/12",
            "10.32.0.0/11",
            "10.64.0.0/10",
            "10.128.0.0/9",
            "10.200.0.0/16",
        ],
        "the /8 keeps its non-ignored fragments, the ignored /24 vanishes, and the \
         disjoint /16 is untouched"
    );
    assert_eq!(
        partial.devices[0].ipv6,
        vec!["fc00::/8"],
        "the v6 mapping is subtracted too: fc00::/7 loses the ignored fd00::/8"
    );
    assert!(
        !inputs
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "ignored-1"),
        "a device whose only mapping is ignored is dropped"
    );
}

/// A3, the over-prune half: a mapping whose NETWORK ADDRESS falls inside an
/// ignore but whose prefix extends beyond it is no longer dropped whole — old
/// code compared the address, so it lost the permitted majority of the prefix.
#[test]
fn a_device_prefix_whose_address_is_ignored_keeps_its_broader_coverage() {
    let mut config = tunnel_test_config("u7-device-prefix-over-prune");
    config.ip_ranges.ignore_subnets = vec!["10.1.0.0/16".to_string()];
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    fs::write(
        ConfigShapedDevices::path_for_config(&config),
        concat!(
            "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,",
            "Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,",
            "Download Max Mbps,Upload Max Mbps,Comment\n",
            "\"broad-1\",\"Broad\",\"broad-dev\",\"Broad\",\"site-1\",\"\",\"\",",
            "\"aa:bb:cc:dd:ee:03\",\"10.1.0.0/9\",\"\",10,5,100,50,\"\"\n",
        ),
    )
    .expect("the provisioned csv should write");

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let broad = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "broad-1")
        .expect("the broader mapping must survive");
    assert_eq!(
        broad.devices[0].ipv4,
        vec![
            "10.0.0.0/16",
            "10.2.0.0/15",
            "10.4.0.0/14",
            "10.8.0.0/13",
            "10.16.0.0/12",
            "10.32.0.0/11",
            "10.64.0.0/10",
        ],
        "10.1.0.0/9 covers 10.0.0.0/9, so only the ignored /16 is removed"
    );
}
