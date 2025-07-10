use std::time::Instant;

use allocative::Allocative;

#[derive(PartialEq, Debug, Allocative)]
pub enum StormguardState {
    Warmup,
    Running,
    Cooldown{ start: Instant, duration_secs: f32 },
}