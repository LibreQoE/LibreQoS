use parking_lot::Mutex;

#[derive(Clone)]
pub(crate) struct LicenseStatus {
    pub(crate) license_type: i32,
    pub(crate) trial_expires: i32,
}

impl Default for LicenseStatus {
    fn default() -> Self {
        LicenseStatus {
            license_type: -1,
            trial_expires: -1,
        }
    }
}

static LICENSE_STATE: Mutex<LicenseStatus> = Mutex::new(LicenseStatus {
    license_type: -1,
    trial_expires: -1,
});

pub fn set_license_status(status: LicenseStatus) {
    let mut lock = LICENSE_STATE.lock();
    *lock = status;
}

pub fn get_license_status() -> LicenseStatus {
    let lock = LICENSE_STATE.lock();
    (*lock).clone()
}