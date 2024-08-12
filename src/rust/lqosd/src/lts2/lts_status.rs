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
            1 => LtsStatus::AlwaysFree,
            2 => LtsStatus::FreeTrial,
            3 => LtsStatus::SelfHosted,
            4 => LtsStatus::ApiOnly,
            5 => LtsStatus::Full,
            _ => LtsStatus::Invalid,
        }
    }
}
