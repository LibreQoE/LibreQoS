use crate::lts2_sys::RemoteCommand;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

static COMMAND_LIST: Lazy<Mutex<Vec<RemoteCommand>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub fn enqueue(command: Vec<RemoteCommand>) {
    let mut list = COMMAND_LIST.lock();
    list.extend(command);
}

pub fn count() -> usize {
    let list = COMMAND_LIST.lock();
    list.len()
}

pub fn get() -> Vec<RemoteCommand> {
    let mut list = COMMAND_LIST.lock();
    let mut result = Vec::new();
    std::mem::swap(&mut result, &mut *list);
    result
}
