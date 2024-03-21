/// Generic RingBuffer Type for storing statistics
pub struct StatsRingBuffer<T, const N: usize> {
    buffer: Vec<T>,
    index: usize,
}

impl <T: Default + Clone, const N: usize> StatsRingBuffer<T, N> {
    /// Creates an empty ringbuffer
    pub fn new() -> Self {
        Self {
            buffer: vec![T::default(); N],
            index: 0,
        }
    }

    /// Adds an entry to the ringbuffer
    pub fn push(&mut self, value: T) {
        self.buffer[self.index] = value;
        self.index = (self.index + 1) % N;
    }

    /// Retrieves a clone of the ringbuffer, in the order the values were added
    pub fn get_values_in_order(&self) -> Vec<T> {
        let mut result = Vec::with_capacity(N);

        for i in self.index..N {
            result.push(self.buffer[i].clone());
        }
        for i in 0..self.index {
            result.push(self.buffer[i].clone());
        }

        result
    }
}