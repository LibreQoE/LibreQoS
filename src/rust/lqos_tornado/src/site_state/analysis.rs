use std::fmt::Display;
use crate::site_state::ring_buffer::RingBuffer;

#[derive(PartialEq)]
pub enum SaturationLevel {
    Low,
    Medium,
    High,
}

impl Display for SaturationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaturationLevel::Low => write!(f, "Low"),
            SaturationLevel::Medium => write!(f, "Medium"),
            SaturationLevel::High => write!(f, "High"),
        }
    }
}

impl SaturationLevel {
    pub fn from_throughput(value: f64, max: f64) -> Self {
        if value < max * 0.5 {
            SaturationLevel::Low
        } else if value < max * 0.85 {
            SaturationLevel::Medium
        } else {
            SaturationLevel::High
        }
    }
}

#[derive(PartialEq)]
pub enum RetransmitState {
    RisingFast,
    Rising,
    Stable,
    Falling,
    FallingFast,
}

impl Display for RetransmitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RetransmitState::RisingFast => write!(f, "RisingFast"),
            RetransmitState::Rising => write!(f, "Rising"),
            RetransmitState::Stable => write!(f, "Stable"),
            RetransmitState::Falling => write!(f, "Falling"),
            RetransmitState::FallingFast => write!(f, "FallingFast"),
        }
    }
}

impl RetransmitState {
    pub fn new(
        moving_average: &RingBuffer,
        recent: &RingBuffer,
    ) -> Self {
        let tcp_retransmits_ma = moving_average.average().unwrap_or(0.01);
        let tcp_retransmits_avg = recent.average().unwrap_or(0.01);
        let tcp_retransmits_relative = tcp_retransmits_avg / tcp_retransmits_ma;
        
        // Determine State
        if tcp_retransmits_relative < 0.4 {
            RetransmitState::FallingFast
        } else if tcp_retransmits_relative < 0.8 {
            RetransmitState::Falling
        }  else if tcp_retransmits_relative > 1.8 {
            RetransmitState::RisingFast
        } else if tcp_retransmits_relative > 1.2 {
            RetransmitState::Rising
        } else {
            RetransmitState::Stable
        }
    }
}

#[derive(PartialEq)]
pub enum RttState {
    Rising{magnitude: f32},
    Flat,
    Falling{magnitude: f32},
}

impl Display for RttState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RttState::Rising{magnitude} => write!(f, "Rising ({magnitude:.2})"),
            RttState::Flat => write!(f, "Flat"),
            RttState::Falling{magnitude} => write!(f, "Falling ({magnitude:.2})"),
        }
    }
}

impl RttState {
    pub fn new(
        moving_average: &RingBuffer,
        recent: &RingBuffer,
    ) -> Self {
        if recent.count() < 2 || moving_average.count() < 2 {
            return RttState::Flat;
        }
        let rtt_ma = moving_average.average().unwrap_or(1.0);
        let rtt_avg = recent.average().unwrap_or(1.0);
        let rtt_relative = rtt_avg / rtt_ma;
        let delta = (rtt_relative - 1.0).abs() as f32;

        // Determine State
        if rtt_relative > 1.2 {
            RttState::Rising {magnitude: delta}
        } else if rtt_relative < 0.8 {
            RttState::Falling { magnitude: delta}
        } else {
            RttState::Flat
        }
    }
}