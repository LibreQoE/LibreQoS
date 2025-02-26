#[derive(Default)]
pub struct SizeInfo {
    pub raw_size: u64,
    pub final_size: u64,
}

impl std::ops::AddAssign for SizeInfo {
    fn add_assign(&mut self, other: Self) {
        self.raw_size += other.raw_size;
        self.final_size += other.final_size;
    }
}
