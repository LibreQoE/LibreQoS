use std::sync::Mutex;
use lqos_bus::BusResponse;
use once_cell::sync::Lazy;

pub static THROUGHPUT_RING: Lazy<Mutex<ThroughputRingbuffer>> = Lazy::new(|| Mutex::new(ThroughputRingbuffer::default()));
const RINGBUFFER_SIZE: usize = 80;
pub static CURRENT_THROUGHPUT: Lazy<Mutex<CurrentThroughput>> = Lazy::new(|| Mutex::new(CurrentThroughput::default()));

#[derive(Default, Copy, Clone)]
pub struct CurrentThroughput {
    pub bits_per_second: (u64, u64),
    pub packets_per_second: (u64, u64),
    pub shaped_bits_per_second: (u64, u64),
}

pub struct ThroughputRingbuffer {
    current_index: usize,
    pub ringbuffer: [CurrentThroughput; RINGBUFFER_SIZE],
}

impl ThroughputRingbuffer {
    fn push(&mut self, current: CurrentThroughput) {
        self.ringbuffer[self.current_index] = current;
        self.current_index = (self.current_index + 1) % RINGBUFFER_SIZE;
    }

    pub fn bits_per_second_vec_up(&self) -> Vec<u64> {
        let mut result = Vec::with_capacity(RINGBUFFER_SIZE);

        for i in self.current_index..RINGBUFFER_SIZE {
            result.push(self.ringbuffer[i].bits_per_second.0);
        }
        for i in 0..self.current_index {
            result.push(self.ringbuffer[i].bits_per_second.0);
        }

        result
    }

    pub fn bits_per_second_vec_down(&self) -> Vec<u64> {
        let mut result = Vec::with_capacity(RINGBUFFER_SIZE);

        for i in self.current_index..RINGBUFFER_SIZE {
            result.push(self.ringbuffer[i].bits_per_second.1);
        }
        for i in 0..self.current_index {
            result.push(self.ringbuffer[i].bits_per_second.1);
        }

        result
    }

    pub fn shaped_bits_per_second_vec_up(&self) -> Vec<u64> {
        let mut result = Vec::with_capacity(RINGBUFFER_SIZE);

        for i in self.current_index..RINGBUFFER_SIZE {
            result.push(self.ringbuffer[i].shaped_bits_per_second.0);
        }
        for i in 0..self.current_index {
            result.push(self.ringbuffer[i].shaped_bits_per_second.0);
        }

        result
    }

    pub fn shaped_bits_per_second_vec_down(&self) -> Vec<u64> {
        let mut result = Vec::with_capacity(RINGBUFFER_SIZE);

        for i in self.current_index..RINGBUFFER_SIZE {
            result.push(self.ringbuffer[i].shaped_bits_per_second.1);
        }
        for i in 0..self.current_index {
            result.push(self.ringbuffer[i].shaped_bits_per_second.1);
        }

        result
    }
}

impl Default for ThroughputRingbuffer {
    fn default() -> Self {
        let mut ringbuffer = [CurrentThroughput::default(); RINGBUFFER_SIZE];
        for i in 0..RINGBUFFER_SIZE {
            ringbuffer[i].bits_per_second = (0, 0);
            ringbuffer[i].packets_per_second = (0, 0);
            ringbuffer[i].shaped_bits_per_second = (0, 0);
        }
        ThroughputRingbuffer {
            current_index: 0,
            ringbuffer,
        }
    }
}

pub async fn throughput(response: &BusResponse) {
    if let BusResponse::CurrentThroughput {
        bits_per_second,
        packets_per_second,
        shaped_bits_per_second,
    } = response
    {
        let mut rb = THROUGHPUT_RING.lock().unwrap();
        rb.push(CurrentThroughput {
            bits_per_second: *bits_per_second,
            packets_per_second: *packets_per_second,
            shaped_bits_per_second: *shaped_bits_per_second,
        });

        let mut current = CURRENT_THROUGHPUT.lock().unwrap();
        current.bits_per_second = *bits_per_second;
        current.packets_per_second = *packets_per_second;
        current.shaped_bits_per_second = *shaped_bits_per_second;        
    }
}
