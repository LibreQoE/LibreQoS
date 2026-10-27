//! The v2→v3 pre-install qualification core
//! (`docs/v2_to_v3_preflight_card.md`).
//!
//! Pure and daemon-free by construction: it is shared by two surfaces —
//! the standalone `libreqos-v3-preflight` artifact (which must run on a v2
//! box that has no v3 install) and the v3 deb's `preinst` gate (compiled
//! into the package). No systemd, no daemon state; the standalone binary
//! reads the host config only for the dataplane pair (the netplan arm).

pub mod bindplan;
pub mod circuits;
pub mod hardware;
pub mod install;
pub mod netplan;
pub mod os;
pub mod prereq;
pub mod seam;
pub mod topology;
pub mod units;
pub mod verdict;

pub use bindplan::{BindMode, NicPlan};
pub use circuits::count_shaped_devices;
pub use hardware::{HardwareClass, HardwareVerdict, HugePages, virtio_qualified};
pub use install::{InstallIntegrityReport, INTEGRITY_SET};
pub use netplan::{NetplanReport, on_a_stick_bridge_conf, pair_from_config};
pub use os::{OsRelease, UBUNTU_FLOOR};
pub use prereq::{DEFAULT_DATAPLANE_URL, FetchPrereq, RepoConfig, check_fetch, check_repo};
pub use seam::{EXPECTED_PROTOCOL_VERSION, INSTALLED_LQOSD, SeamReport};
pub use topology::{CompileReport, validate_pair, validate_pair_with_mode};
pub use units::{LEGACY_V2_UNITS, UnitPresence, UnitRow, inventory};
pub use verdict::{Severity, Verdict, compose, compose_with_fresh, compose_with_mode};
