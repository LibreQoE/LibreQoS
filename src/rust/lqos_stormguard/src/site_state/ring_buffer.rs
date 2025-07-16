use allocative::Allocative;

#[derive(Allocative)]
pub struct RingBuffer {
    data: Vec<Option<f64>>,
    index: usize,
}

impl RingBuffer {
    pub fn new(size: usize) -> Self {
        RingBuffer {
            data: vec![None; size],
            index: 0,
        }
    }

    pub fn add(&mut self, item: f64) {
        self.data[self.index] = Some(item);
        self.index = (self.index + 1) % self.data.len();
    }
    
    pub fn count(&self) -> usize {
        self.data.iter().filter(|x| x.is_some()).count()
    }
    
    pub fn average(&self) -> Option<f64> {
        let count = self.count();
        if count == 0 {
            return None;
        }
        let sum: f64 = self.data.iter().filter_map(|x| *x).sum();
        let count = count as f64;
        if count > 0.0 {
            Some(sum / count)
        } else {
            None
        }
    }
}