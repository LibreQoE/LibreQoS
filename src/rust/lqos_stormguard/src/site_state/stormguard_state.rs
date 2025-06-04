use std::time::Instant;

#[derive(PartialEq, Debug)]
pub enum StormguardState {
    Warmup,
    Running,
    Cooldown{ start: Instant, duration_secs: f32 },
}