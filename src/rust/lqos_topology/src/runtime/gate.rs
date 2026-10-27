use crate::AttachmentProbeSpec;

#[derive(Clone, Debug, Default)]
pub(super) struct RuntimeBuildGate {
    pub(super) last_source_generation: Option<String>,
    pub(super) last_health_effective_signature: Option<u64>,
    pub(super) cached_probe_specs: Vec<AttachmentProbeSpec>,
    pub(super) publish_completed: bool,
    pub(super) next_error_retry_after_unix: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RoundHints {
    pub(super) probes_enabled: bool,
}
