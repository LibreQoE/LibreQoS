//! The walk's credential vocabulary: where candidates came from, one
//! candidate per protocol, the per-node candidate lists, and the order the
//! walker tries them in.
//!
//! It sits beside the walk core rather than inside it: these types name
//! credentials and their precedence only. The walker owns probing and the
//! sources own resolution; nothing here reads a vault or opens a socket.
//!
//! Candidate values carry their own redacted `Debug`, so a candidate and a
//! node's credential list are safe to name in logs. The origin label is
//! non-secret by construction.

use std::fmt;

use super::{ApiCredentials, CredentialClass, SnmpAccess};

/// Where one credential candidate came from. Non-secret by construction,
/// so it is safe in `Debug` output and in logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialOrigin {
    /// The seed target's own credential: the walk's initial default.
    Seed,
    /// A vault target's explicit `credential` or `api_credential` ref.
    Target,
    /// A discovery profile, named.
    Profile(String),
}

impl fmt::Display for CredentialOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seed => f.write_str("seed"),
            Self::Target => f.write_str("target"),
            Self::Profile(name) => f.write_str(name),
        }
    }
}

/// One credential the walker may try against a node, with its precedence
/// class and the non-secret label of where it came from.
///
/// `Debug` prints the class, the origin, and the credential's own redacted
/// `Debug`: no community, user name, or password reaches the output.
#[derive(Debug)]
pub struct CredentialCandidate<T> {
    /// The precedence class this candidate is ordered by.
    pub class: CredentialClass,
    /// Where the credential came from; safe to log.
    pub origin: CredentialOrigin,
    /// The credential itself, redacted by its own `Debug`.
    pub credential: T,
}

impl<T> CredentialCandidate<T> {
    /// A credential candidate with its precedence class and origin label.
    pub fn new(class: CredentialClass, origin: CredentialOrigin, credential: T) -> Self {
        Self {
            class,
            origin,
            credential,
        }
    }
}

/// The credential candidates the walker may try against one node, per
/// protocol, in class order. A protocol with no candidate is not probed.
///
/// Candidates arrive ordered by [`CredentialClass`] — the seed target's
/// credential, then default profiles, then target refs, then scoped
/// profiles — with file order inside a class. The live source may hoist a
/// profile it already saw answer, but only within the default or scoped
/// class; the seed stays first and target refs keep their file order, so
/// nothing crosses a class boundary.
#[derive(Debug)]
pub struct NodeCredentials {
    /// SNMP candidates for the LLDP feed and the SNMP details fallback.
    pub snmp: Vec<CredentialCandidate<SnmpAccess>>,
    /// RouterOS API candidates for the neighbour feed and the node's
    /// details.
    pub api: Vec<CredentialCandidate<ApiCredentials>>,
}

impl NodeCredentials {
    /// Credentials for nothing: both feeds unavailable.
    pub fn none() -> Self {
        Self {
            snmp: Vec::new(),
            api: Vec::new(),
        }
    }

    /// Adds a target-ref SNMP candidate after the ones already present.
    pub fn with_target_snmp(mut self, access: SnmpAccess) -> Self {
        self.snmp.push(CredentialCandidate::new(
            CredentialClass::Target,
            CredentialOrigin::Target,
            access,
        ));
        self
    }

    /// Adds a target-ref RouterOS API candidate after the ones already
    /// present.
    pub fn with_target_api(mut self, credentials: ApiCredentials) -> Self {
        self.api.push(CredentialCandidate::new(
            CredentialClass::Target,
            CredentialOrigin::Target,
            credentials,
        ));
        self
    }
}

/// Candidates in the order the walker tries them for one node: seed, then
/// default profiles, then target refs, then scoped profiles, preserving
/// candidate order inside each class. `preferred` is hoisted to the front
/// of its own class only — a default profile ahead of the other defaults, a
/// scoped profile ahead of the other scoped profiles — so the seed stays
/// first, target refs keep file order, and nothing crosses a class.
pub(crate) fn attempt_order<'a, T>(
    candidates: &'a [CredentialCandidate<T>],
    preferred: Option<&CredentialOrigin>,
) -> Vec<&'a CredentialCandidate<T>> {
    let mut ordered: Vec<&CredentialCandidate<T>> = Vec::with_capacity(candidates.len());
    for class in [
        CredentialClass::Seed,
        CredentialClass::Default,
        CredentialClass::Target,
        CredentialClass::Scoped,
    ] {
        let in_class = || {
            candidates
                .iter()
                .filter(move |candidate| candidate.class == class)
        };
        if let Some(preferred) = preferred
            && matches!(class, CredentialClass::Default | CredentialClass::Scoped)
        {
            ordered.extend(in_class().filter(|candidate| &candidate.origin == preferred));
            ordered.extend(in_class().filter(|candidate| &candidate.origin != preferred));
            continue;
        }
        ordered.extend(in_class());
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::mikrotik::Password;
    use crate::snmp::{Community, SnmpVersion};

    /// An SNMP candidate with its class and origin label.
    fn candidate(
        class: CredentialClass,
        origin: CredentialOrigin,
        community: &str,
    ) -> CredentialCandidate<SnmpAccess> {
        CredentialCandidate::new(
            class,
            origin,
            SnmpAccess {
                community: Community::new(community.to_owned()),
                version: SnmpVersion::V2c,
            },
        )
    }

    /// The attempt order's origin labels.
    fn order(ordered: &[&CredentialCandidate<SnmpAccess>]) -> Vec<String> {
        ordered
            .iter()
            .map(|candidate| candidate.origin.to_string())
            .collect()
    }

    /// One candidate per class, in a deliberately shuffled input order: the
    /// seed credential, two defaults, one target ref, two scoped profiles.
    fn every_class() -> Vec<CredentialCandidate<SnmpAccess>> {
        vec![
            candidate(
                CredentialClass::Scoped,
                CredentialOrigin::Profile("exact".to_owned()),
                "exact-community",
            ),
            candidate(
                CredentialClass::Target,
                CredentialOrigin::Target,
                "target-community",
            ),
            candidate(
                CredentialClass::Default,
                CredentialOrigin::Profile("shared".to_owned()),
                "shared-community",
            ),
            candidate(
                CredentialClass::Seed,
                CredentialOrigin::Seed,
                "seed-community",
            ),
            candidate(
                CredentialClass::Scoped,
                CredentialOrigin::Profile("wide".to_owned()),
                "wide-community",
            ),
            candidate(
                CredentialClass::Default,
                CredentialOrigin::Profile("office".to_owned()),
                "office-community",
            ),
        ]
    }

    #[test]
    fn classes_are_tried_seed_then_default_then_target_then_scoped() {
        assert_eq!(
            order(&attempt_order(&every_class(), None)),
            ["seed", "shared", "office", "target", "exact", "wide"],
            "seed first, then defaults and scoped profiles in their resolution \
             order, with target refs between them"
        );
    }

    #[test]
    fn a_preferred_default_is_hoisted_only_among_the_defaults() {
        let preferred = CredentialOrigin::Profile("office".to_owned());
        assert_eq!(
            order(&attempt_order(&every_class(), Some(&preferred))),
            ["seed", "office", "shared", "target", "exact", "wide"],
            "the default winner moves ahead of the other defaults and no \
             further: the seed stays first and the target ref keeps its place"
        );
    }

    #[test]
    fn a_preferred_scoped_profile_is_hoisted_only_among_the_scoped() {
        let preferred = CredentialOrigin::Profile("wide".to_owned());
        assert_eq!(
            order(&attempt_order(&every_class(), Some(&preferred))),
            ["seed", "shared", "office", "target", "wide", "exact"],
            "the scoped winner moves ahead of the other scoped profiles only"
        );
    }

    #[test]
    fn a_preferred_target_or_seed_never_reorders_anything() {
        // Target refs and the seed are never hoisted; a preference for one
        // of them is a no-op because both classes already lead in their
        // place.
        for preferred in [CredentialOrigin::Target, CredentialOrigin::Seed] {
            assert_eq!(
                order(&attempt_order(&every_class(), Some(&preferred))),
                ["seed", "shared", "office", "target", "exact", "wide"],
                "a {preferred} preference changes no class order"
            );
        }
    }

    #[test]
    fn a_preferred_credential_the_node_cannot_use_changes_nothing() {
        let elsewhere = CredentialOrigin::Profile("other-scope".to_owned());
        assert_eq!(
            order(&attempt_order(&every_class(), Some(&elsewhere))),
            ["seed", "shared", "office", "target", "exact", "wide"],
            "a profile this node does not carry is never hoisted"
        );
    }

    #[test]
    fn credentials_stay_redacted_while_origins_print() {
        let credentials = NodeCredentials::none()
            .with_target_snmp(SnmpAccess {
                community: Community::new("sentinel-community-1".to_owned()),
                version: SnmpVersion::V2c,
            })
            .with_target_api(ApiCredentials {
                username: "sentinel-user".to_owned(),
                password: Password::new("sentinel-pass".to_owned()),
            });
        let debug = format!("{credentials:?}");
        for secret in ["sentinel-community-1", "sentinel-user", "sentinel-pass"] {
            assert!(!debug.contains(secret), "secret leaked into Debug: {debug}");
        }
        assert!(
            debug.contains("class: Target") && debug.contains("origin: Target"),
            "the class and origin labels stay printable: {debug}"
        );
    }

    #[test]
    fn a_single_candidate_keeps_its_order() {
        let candidates = [candidate(
            CredentialClass::Target,
            CredentialOrigin::Target,
            "only-community",
        )];
        let preferred = CredentialOrigin::Profile("exact".to_owned());
        assert_eq!(
            order(&attempt_order(&candidates, Some(&preferred))),
            ["target"]
        );
    }
}
