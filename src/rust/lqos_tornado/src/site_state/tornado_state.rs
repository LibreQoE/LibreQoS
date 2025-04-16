use std::time::Instant;

#[derive(PartialEq, Debug)]
pub enum TornadoState {
    Warmup,
    Running,
    Cooldown(Instant)
}