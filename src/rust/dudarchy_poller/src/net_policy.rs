//! The scan policy core: a pure, I/O-free value that decides whether
//! one address may be probed.
//!
//! A [`ScanPolicy`] holds allow and ignore ranges spanning IPv4 and
//! IPv6. Both families match per family — an IPv4 rule never admits an
//! IPv6 address and vice versa. Ignore always wins over allow on
//! overlap: an address matching any ignore range is ignored before the
//! allow rules are consulted. Otherwise the longest-prefix (most
//! specific) matching allow rule decides, and the decision carries that
//! rule.
//!
//! Rules are CIDR strings (`10.0.0.0/8`, `fc00::/7`) or bare addresses,
//! which ipnet accepts as their host routes (`/32`, `/128`); host bits
//! written into a CIDR are handled by ipnet's network semantics. A
//! policy with an empty allow list is a valid value that allows
//! nothing — the configuration layer owns whether that is acceptable.
//!
//! Pure and deterministic: no vault, no logging, no network.

use std::fmt;
use std::net::IpAddr;

use ipnet::IpNet;

/// Which list a rule came from, named by parse errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleList {
    /// Rules an address must match to be probed.
    Allow,
    /// Rules that exclude an address even when an allow rule matches.
    Ignore,
}

impl fmt::Display for RuleList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Allow => "allow",
            Self::Ignore => "ignore",
        })
    }
}

/// One rule string failed to parse: the offending text, the list it
/// came from, and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanPolicyError {
    /// The rule text that failed to parse.
    pub text: String,
    /// The list the rule came from.
    pub list: RuleList,
    /// What was wrong with it.
    pub reason: String,
}

impl fmt::Display for ScanPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid {} rule `{}`: {}",
            self.list, self.text, self.reason
        )
    }
}

impl std::error::Error for ScanPolicyError {}

/// Whether one address may be probed, and which rule decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanDecision {
    /// The address matches the allow list. The deciding rule is the
    /// most specific matching allow rule.
    Allowed {
        /// The longest-prefix matching allow rule.
        rule: Option<IpNet>,
    },
    /// The address matches the ignore list — ignore wins over allow on
    /// any overlap. The rule is the most specific matching ignore rule.
    Ignored {
        /// The longest-prefix matching ignore rule.
        rule: IpNet,
    },
    /// The address matches no allow rule.
    NotAllowed,
}

/// The allow/ignore ranges a probe checks one address against.
///
/// Constructed from rule strings; the lists keep their given order and
/// duplicates, and `decide` scans them linearly — the sizes a scan
/// policy carries make anything smarter unnecessary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanPolicy {
    allow: Vec<IpNet>,
    ignore: Vec<IpNet>,
}

impl ScanPolicy {
    /// Parses the allow and ignore rule strings into a policy. A bare
    /// address is accepted as its host route (`/32` for IPv4, `/128`
    /// for IPv6); host bits written into a CIDR are handled by ipnet's
    /// network semantics. The first invalid rule fails the
    /// construction, naming its text and list.
    pub fn new(allow: &[String], ignore: &[String]) -> Result<Self, ScanPolicyError> {
        let allow = allow
            .iter()
            .map(|text| parse_rule(text, RuleList::Allow))
            .collect::<Result<Vec<_>, _>>()?;
        let ignore = ignore
            .iter()
            .map(|text| parse_rule(text, RuleList::Ignore))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { allow, ignore })
    }

    /// Decides whether `address` may be probed. An empty allow list
    /// allows nothing; ignore rules are consulted first and win on any
    /// overlap; otherwise the most specific matching allow rule decides.
    pub fn decide(&self, address: IpAddr) -> ScanDecision {
        // `contains` is family-checked, so an IPv4 rule never matches an
        // IPv6 address and vice versa.
        if let Some(rule) = longest_match(&self.ignore, address) {
            return ScanDecision::Ignored { rule };
        }
        match longest_match(&self.allow, address) {
            Some(rule) => ScanDecision::Allowed { rule: Some(rule) },
            None => ScanDecision::NotAllowed,
        }
    }

    /// How many allow rules the policy holds.
    pub fn allow_count(&self) -> usize {
        self.allow.len()
    }

    /// How many ignore rules the policy holds.
    pub fn ignore_count(&self) -> usize {
        self.ignore.len()
    }
}

/// The longest-prefix (most specific) rule containing `address`, if
/// any. Prefix lengths only compare within one family, which is exactly
/// the per-family match the policy requires.
fn longest_match(rules: &[IpNet], address: IpAddr) -> Option<IpNet> {
    rules
        .iter()
        .filter(|net| net.contains(&address))
        .max_by_key(|net| net.prefix_len())
        .copied()
}

/// Parses one rule string: a CIDR range, or a bare address as its host
/// route.
fn parse_rule(text: &str, list: RuleList) -> Result<IpNet, ScanPolicyError> {
    let fail = |reason: String| ScanPolicyError {
        text: text.to_owned(),
        list,
        reason,
    };
    match text.parse::<IpNet>() {
        Ok(net) => Ok(net),
        Err(_) => text
            .parse::<IpAddr>()
            .map(IpNet::from)
            .map_err(|_| fail("not a CIDR range or an IP address".to_owned())),
    }
}

/// One address string after canonicalization: the address the scan
/// policy judges, and the probe-facing text the transport contacts.
///
/// The two are the same address by construction — the probe text either
/// is the judged address's canonical form or adds a zone scope to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalAddress {
    /// The bare address the scan policy judges: IPv4-mapped IPv6
    /// reduced to IPv4, zones removed, compressed lowercase otherwise.
    judged: IpAddr,
    /// The probe-facing text: the judged address's canonical form, plus
    /// a preserved zone scope when the input carried one.
    probe: String,
}

impl CanonicalAddress {
    /// The bare address the scan policy judges.
    pub fn judged(&self) -> IpAddr {
        self.judged
    }

    /// The text a transport contacts: the judged address's canonical
    /// form, with the zone scope preserved when the input carried one.
    pub fn probe_text(&self) -> &str {
        &self.probe
    }
}

/// One address string failed canonicalization: the offending text and
/// the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressError {
    /// The address text that failed.
    pub text: String,
    /// What was wrong with it.
    pub reason: String,
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid address `{}`: {}", self.text, self.reason)
    }
}

impl std::error::Error for AddressError {}

/// Canonicalizes one address string — an LLDP or RouterOS neighbour
/// address, or a vault seed address — into the pair the scanner works
/// with: the bare [`IpAddr`] the scan policy judges, and the
/// probe-facing text a transport contacts.
///
/// Surrounding whitespace is trimmed; IPv4-mapped IPv6
/// (`::ffff:192.168.0.1`, any case) is judged and probed as the IPv4
/// address so an IPv4 ignore cannot be bypassed; a zone scope
/// (`fe80::1%eth0`) is policy data only for the bare address and is
/// preserved just in the probe text; IPv6 normalizes to compressed
/// lowercase. Anything that is not an IP literal — hostnames,
/// `host:port` pairs, empty strings, an empty zone — is refused: there
/// is no DNS resolution, fail closed.
pub fn canonicalize(text: &str) -> Result<CanonicalAddress, AddressError> {
    let fail = |reason: &str| AddressError {
        text: text.to_owned(),
        reason: reason.to_owned(),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(fail("empty address"));
    }
    // A zone scope is transport data, never policy data: split it off
    // before parsing — std rejects zones in `IpAddr::from_str`.
    let (address_part, zone) = match trimmed.split_once('%') {
        Some((address, zone)) => (address, Some(zone)),
        None => (trimmed, None),
    };
    let Some(zone) = zone else {
        let judged = parse_judged(address_part, &fail)?;
        let probe = judged.to_string();
        return Ok(CanonicalAddress { judged, probe });
    };
    if zone.is_empty() {
        return Err(fail("empty zone scope"));
    }
    if zone.contains('%') {
        return Err(fail("multiple zone scopes"));
    }
    let judged = parse_judged(address_part, &fail)?;
    let probe = format!("{judged}%{zone}");
    Ok(CanonicalAddress { judged, probe })
}

/// Parses the address part into the judged, canonical `IpAddr`:
/// IPv4-mapped IPv6 reduced to IPv4, compressed lowercase otherwise.
fn parse_judged(text: &str, fail: &dyn Fn(&str) -> AddressError) -> Result<IpAddr, AddressError> {
    text.parse::<IpAddr>()
        .map(|addr| addr.to_canonical())
        .map_err(|_| fail("not an IPv4 or IPv6 address"))
}

/// The address gate a walk applies to every candidate: the optional
/// scan policy bundled with the canonicalization it runs. Call sites
/// pass one value; the judging logic lives here.
#[derive(Clone, Copy, Debug, Default)]
pub struct AddressGate<'a> {
    policy: Option<&'a ScanPolicy>,
}

impl<'a> AddressGate<'a> {
    /// An unrestricted gate: every parseable address is admitted.
    pub fn unrestricted() -> Self {
        Self { policy: None }
    }

    /// A gate backed by a scan policy.
    pub fn new(policy: Option<&'a ScanPolicy>) -> AddressGate<'a> {
        Self { policy }
    }

    /// Canonicalizes and judges one candidate address text. Every
    /// verdict carries the candidate: `Admit` is the one to store and
    /// probe; `Ignored` and `NotAllowed` are refusals whose canonical
    /// address is for the refusal report. An unparseable address is an
    /// error only under an enforcing gate — an unrestricted gate admits
    /// it verbatim, matching today's no-policy behavior.
    pub fn judge(&self, text: &str) -> Result<GateVerdict, AddressError> {
        let trimmed = text.trim();
        let Some(policy) = self.policy else {
            return Ok(GateVerdict::Admit(JudgedAddress {
                address: trimmed.to_owned(),
                judged: None,
                rule: None,
            }));
        };
        let canonical = canonicalize(trimmed)?;
        let judged = |rule: Option<IpNet>| JudgedAddress {
            address: canonical.probe_text().to_owned(),
            judged: Some(canonical.judged()),
            rule,
        };
        Ok(match policy.decide(canonical.judged()) {
            ScanDecision::Allowed { rule } => GateVerdict::Admit(judged(rule)),
            ScanDecision::Ignored { rule } => GateVerdict::Ignored(judged(Some(rule))),
            ScanDecision::NotAllowed => GateVerdict::NotAllowed(judged(None)),
        })
    }
}

/// One candidate address after the gate: the text to store and probe,
/// the bare IP the policy judged — `None` when the gate ran
/// unrestricted and nothing judged the address — and the rule that
/// decided the verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JudgedAddress {
    /// The address to store and probe: canonical when a policy judged
    /// it, else the trimmed raw text.
    pub address: String,
    /// The bare judged IP, when a policy judged the address.
    pub judged: Option<IpAddr>,
    /// The rule that decided the verdict: the deciding ignore rule for
    /// an ignored address, the matching allow rule for an admitted one,
    /// `None` when nothing matched or the gate is unrestricted.
    pub rule: Option<IpNet>,
}

/// The verdict for one candidate address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateVerdict {
    /// The address matches the allow list (or the gate is
    /// unrestricted): store and probe it.
    Admit(JudgedAddress),
    /// The address is matched by an ignore rule — ignore wins over
    /// allow.
    Ignored(JudgedAddress),
    /// The address is outside every allow rule.
    NotAllowed(JudgedAddress),
}

impl GateVerdict {
    /// The candidate's stored address, whatever the verdict.
    pub fn address(&self) -> &str {
        match self {
            Self::Admit(admitted) | Self::Ignored(admitted) | Self::NotAllowed(admitted) => {
                &admitted.address
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a policy from rule strings, panicking on fixture typos.
    fn policy(allow: &[&str], ignore: &[&str]) -> ScanPolicy {
        let allow: Vec<String> = allow.iter().map(|rule| (*rule).to_owned()).collect();
        let ignore: Vec<String> = ignore.iter().map(|rule| (*rule).to_owned()).collect();
        ScanPolicy::new(&allow, &ignore).expect("fixture rules should parse")
    }

    #[test]
    fn prefix_boundaries_admit_the_network_and_the_last_address_only() {
        let policy = policy(&["10.0.0.0/8"], &[]);
        assert_eq!(
            policy.decide("10.0.0.0".parse().expect("address")),
            ScanDecision::Allowed {
                rule: Some("10.0.0.0/8".parse().expect("rule"))
            }
        );
        assert_eq!(
            policy.decide("10.255.255.255".parse().expect("address")),
            ScanDecision::Allowed {
                rule: Some("10.0.0.0/8".parse().expect("rule"))
            }
        );
        assert_eq!(
            policy.decide("11.0.0.0".parse().expect("address")),
            ScanDecision::NotAllowed
        );
        assert_eq!(
            policy.decide("9.255.255.255".parse().expect("address")),
            ScanDecision::NotAllowed
        );
    }

    #[test]
    fn a_zero_prefix_admits_every_address_of_its_family() {
        let policy = policy(&["0.0.0.0/0", "::/0"], &[]);
        assert!(matches!(
            policy.decide("203.0.113.9".parse().expect("address")),
            ScanDecision::Allowed { .. }
        ));
        assert!(matches!(
            policy.decide("2001:db8::9".parse().expect("address")),
            ScanDecision::Allowed { .. }
        ));
    }

    #[test]
    fn bare_addresses_become_host_routes() {
        let policy = policy(&["192.168.1.5", "2001:db8::1"], &[]);
        assert_eq!(
            policy.decide("192.168.1.5".parse().expect("address")),
            ScanDecision::Allowed {
                rule: Some("192.168.1.5/32".parse().expect("rule"))
            }
        );
        assert_eq!(
            policy.decide("2001:db8::1".parse().expect("address")),
            ScanDecision::Allowed {
                rule: Some("2001:db8::1/128".parse().expect("rule"))
            }
        );
        assert_eq!(
            policy.decide("192.168.1.6".parse().expect("address")),
            ScanDecision::NotAllowed
        );
    }

    #[test]
    fn written_host_bits_are_ipnet_network_semantics() {
        // The written address carries host bits; containment follows the
        // /8 network ipnet derives from it.
        let policy = policy(&["10.0.0.1/8"], &[]);
        assert!(matches!(
            policy.decide("10.9.9.9".parse().expect("address")),
            ScanDecision::Allowed { .. }
        ));
    }

    #[test]
    fn families_match_per_family_only() {
        let v4_only = policy(&["10.0.0.0/8"], &[]);
        assert_eq!(
            v4_only.decide("2001:db8::1".parse().expect("address")),
            ScanDecision::NotAllowed
        );
        let v6_only = policy(&["fc00::/7"], &[]);
        assert_eq!(
            v6_only.decide("10.0.0.1".parse().expect("address")),
            ScanDecision::NotAllowed
        );
    }

    #[test]
    fn ignore_wins_on_exact_and_cross_prefix_overlap() {
        // A narrower ignore beats the wider allow.
        let narrower_ignore = policy(&["10.0.0.0/8"], &["10.1.0.0/16"]);
        assert_eq!(
            narrower_ignore.decide("10.1.2.3".parse().expect("address")),
            ScanDecision::Ignored {
                rule: "10.1.0.0/16".parse().expect("rule")
            }
        );
        // Ignore wins even when the allow rule is the more specific one.
        let wider_ignore = policy(&["10.1.0.0/16"], &["10.0.0.0/8"]);
        assert_eq!(
            wider_ignore.decide("10.1.2.3".parse().expect("address")),
            ScanDecision::Ignored {
                rule: "10.0.0.0/8".parse().expect("rule")
            }
        );
        // An exact-overlap ignore shadows the allow entirely.
        let exact_ignore = policy(&["10.0.0.0/8"], &["10.0.0.0/8"]);
        assert_eq!(
            exact_ignore.decide("10.0.0.1".parse().expect("address")),
            ScanDecision::Ignored {
                rule: "10.0.0.0/8".parse().expect("rule")
            }
        );
        // Outside the ignore range the allow still admits.
        let outside = policy(&["10.0.0.0/8"], &["10.1.0.0/16"]);
        assert!(matches!(
            outside.decide("10.2.0.1".parse().expect("address")),
            ScanDecision::Allowed { .. }
        ));
    }

    #[test]
    fn the_most_specific_matching_rule_decides() {
        let policy = policy(&["10.0.0.0/8", "10.1.0.0/16", "192.168.0.0/16"], &[]);
        assert_eq!(
            policy.decide("10.1.2.3".parse().expect("address")),
            ScanDecision::Allowed {
                rule: Some("10.1.0.0/16".parse().expect("rule"))
            }
        );
        assert_eq!(
            policy.decide("10.2.0.1".parse().expect("address")),
            ScanDecision::Allowed {
                rule: Some("10.0.0.0/8".parse().expect("rule"))
            }
        );
    }

    #[test]
    fn the_most_specific_ignore_rule_is_named() {
        let policy = policy(&["10.0.0.0/8"], &["10.0.0.0/8", "10.1.0.0/16"]);
        assert_eq!(
            policy.decide("10.1.2.3".parse().expect("address")),
            ScanDecision::Ignored {
                rule: "10.1.0.0/16".parse().expect("rule")
            }
        );
    }

    #[test]
    fn the_seeded_private_ranges_admit_their_own_and_refuse_the_internet() {
        let policy = policy(
            &[
                "10.0.0.0/8",
                "172.16.0.0/12",
                "192.168.0.0/16",
                "100.64.0.0/10",
                "fc00::/7",
            ],
            &[],
        );
        for address in [
            "10.20.30.40",
            "172.20.0.1",
            "192.168.100.5",
            "100.64.1.1",
            "fd00::1",
            "fc00::",
        ] {
            assert!(
                matches!(
                    policy.decide(address.parse().expect("address")),
                    ScanDecision::Allowed { .. }
                ),
                "{address} is inside the seeded ranges"
            );
        }
        for address in ["8.8.8.8", "2001:4860::1"] {
            assert_eq!(
                policy.decide(address.parse().expect("address")),
                ScanDecision::NotAllowed,
                "{address} is public and must not be admitted"
            );
        }
    }

    #[test]
    fn an_empty_allow_list_allows_nothing_without_panicking() {
        let policy = ScanPolicy::default();
        assert_eq!(
            policy.decide("10.0.0.1".parse().expect("address")),
            ScanDecision::NotAllowed
        );
        assert_eq!(
            policy.decide("2001:db8::1".parse().expect("address")),
            ScanDecision::NotAllowed
        );
    }

    #[test]
    fn parse_errors_carry_the_text_the_list_and_a_reason() {
        let error =
            ScanPolicy::new(&["10.0.0.0/33".to_owned()], &[]).expect_err("a /33 prefix is invalid");
        assert_eq!(error.text, "10.0.0.0/33");
        assert_eq!(error.list, RuleList::Allow);
        assert!(error.reason.contains("IP address"), "{error}");

        let error =
            ScanPolicy::new(&[], &["banana".to_owned()]).expect_err("a non-CIDR word is invalid");
        assert_eq!(error.text, "banana");
        assert_eq!(error.list, RuleList::Ignore);
        assert_eq!(
            error.to_string(),
            "invalid ignore rule `banana`: not a CIDR range or an IP address"
        );
    }

    // The judged address and the probe text are the same address: the
    // unscoped probe parses back to the judged `IpAddr`; a scoped probe
    // parses back before its `%`.
    fn assert_probe_round_trips(address: &CanonicalAddress) {
        let probe = address.probe_text();
        let address_part = probe.split('%').next().expect("probe text is non-empty");
        assert_eq!(
            address_part
                .parse::<IpAddr>()
                .expect("probe text should parse"),
            address.judged(),
            "the probe text is the judged address"
        );
    }

    #[test]
    fn ipv4_mapped_ipv6_judges_and_probes_as_ipv4() {
        // Any case, with or without the leading zeroes: the judged
        // address is the IPv4 one, so an IPv4 ignore cannot be bypassed.
        for text in [
            "::ffff:192.168.0.1",
            "::FFFF:192.168.0.1",
            "::ffff:C0A8:0001",
        ] {
            let address = canonicalize(text).expect("mapped forms canonicalize");
            assert_eq!(address.judged(), "192.168.0.1".parse::<IpAddr>().unwrap());
            assert_eq!(address.probe_text(), "192.168.0.1");
            assert_probe_round_trips(&address);
        }

        // The judged address really is judged: an IPv4 ignore catches
        // the mapped form.
        let judged_policy = policy(&["192.168.0.0/16"], &["192.168.0.1"]);
        let address = canonicalize("::ffff:192.168.0.1").expect("canonicalizes");
        assert_eq!(
            judged_policy.decide(address.judged()),
            ScanDecision::Ignored {
                rule: "192.168.0.1/32".parse().expect("rule")
            }
        );
        // And an IPv4 allow admits it.
        let allow_all = policy(&["192.168.0.0/16"], &[]);
        assert!(matches!(
            allow_all.decide(address.judged()),
            ScanDecision::Allowed { .. }
        ));
    }

    #[test]
    fn zone_scopes_stay_out_of_policy_and_in_the_probe_text() {
        // Scoped and unscoped judge identically; only the probe text
        // keeps the zone.
        let scoped = canonicalize("fe80::1%eth0").expect("scoped form canonicalizes");
        assert_eq!(scoped.judged(), "fe80::1".parse::<IpAddr>().unwrap());
        assert_eq!(scoped.probe_text(), "fe80::1%eth0");
        assert_probe_round_trips(&scoped);
        let unscoped = canonicalize("fe80::1").expect("unscoped form canonicalizes");
        assert_eq!(unscoped.judged(), scoped.judged());
        assert_eq!(unscoped.probe_text(), "fe80::1");

        // The policy judges the bare address: a link-local ignore range
        // catches the scoped form.
        let policy = policy(&["fe80::/10"], &["fe80::1/128"]);
        assert_eq!(
            policy.decide(scoped.judged()),
            ScanDecision::Ignored {
                rule: "fe80::1/128".parse().expect("rule")
            }
        );

        // An empty zone and a doubled scope are refused.
        let error = canonicalize("fe80::1%").expect_err("an empty zone is refused");
        assert_eq!(error.reason, "empty zone scope");
        let error = canonicalize("fe80::1%eth0%2").expect_err("a doubled scope is refused");
        assert_eq!(error.reason, "multiple zone scopes");
    }

    #[test]
    fn whitespace_and_case_normalize() {
        let address = canonicalize("  FD00::0001  ").expect("whitespace trims");
        assert_eq!(address.judged(), "fd00::1".parse::<IpAddr>().unwrap());
        assert_eq!(address.probe_text(), "fd00::1");
        assert_probe_round_trips(&address);
        let address = canonicalize("\t10.0.0.1\n").expect("whitespace trims");
        assert_eq!(address.judged(), "10.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(address.probe_text(), "10.0.0.1");
    }

    #[test]
    fn non_addresses_fail_closed_with_the_offending_text() {
        let cases = [
            ("", "empty address"),
            ("   ", "empty address"),
            ("example.com", "not an IPv4 or IPv6 address"),
            ("router.local.", "not an IPv4 or IPv6 address"),
            ("10.0.0.1:8080", "not an IPv4 or IPv6 address"),
            ("192.168.0.1/24", "not an IPv4 or IPv6 address"),
            ("[2001:db8::1]:443", "not an IPv4 or IPv6 address"),
        ];
        for (text, reason) in cases {
            let error = canonicalize(text).expect_err(text);
            assert_eq!(error.reason, reason, "{text:?}");
            assert_eq!(
                error.to_string(),
                format!("invalid address `{text}`: {reason}")
            );
        }
    }
}
