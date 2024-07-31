#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
#[repr(i32)]
pub enum LtsStatus {
    NotChecked = -1,
    AlwaysFree = 0,
    FreeTrial = 1,
    SelfHosted = 2,
    ApiOnly = 3,
    Full = 4,
    Invalid = 5,
}

impl LtsStatus {
    pub fn from_i32(value: i32) -> Self {
        match value {
            -1 => LtsStatus::NotChecked,
            0 => LtsStatus::AlwaysFree,
            1 => LtsStatus::FreeTrial,
            2 => LtsStatus::SelfHosted,
            3 => LtsStatus::ApiOnly,
            4 => LtsStatus::Full,
            _ => LtsStatus::Invalid,
        }
    }
}
