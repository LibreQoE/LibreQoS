use crate::site_state::ring_buffer::RingBuffer;

#[derive(PartialEq)]
pub enum SaturationLevel {
    Low,
    Medium,
    High,
}

impl SaturationLevel {
    pub fn from_throughput(value: f64, max: f64) -> Self {
        if value < max * 0.5 {
            SaturationLevel::Low
        } else if value < max * 0.8 {
            SaturationLevel::Medium
        } else {
            SaturationLevel::High
        }
    }
}

#[derive(PartialEq)]
pub enum RetransmitState {
    High,
    RisingFast,
    Rising,
    Stable,
    Falling,
    FallingFast,
    Low,
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
        if tcp_retransmits_avg < 0.03 {
            RetransmitState::Low
        } else if tcp_retransmits_avg > 0.05 {
            RetransmitState::High
        } else if tcp_retransmits_relative < 0.4 {
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
    Rising,
    Flat,
    Falling,
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

        // Determine State
        if rtt_relative > 1.2 {
            RttState::Rising
        } else if rtt_relative < 0.8 {
            RttState::Falling
        } else {
            RttState::Flat
        }
    }
}