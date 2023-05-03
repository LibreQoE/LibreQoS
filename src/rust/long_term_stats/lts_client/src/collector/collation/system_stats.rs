use once_cell::sync::Lazy;
use sysinfo::{System, SystemExt};
use tokio::sync::Mutex;

static SYS: Lazy<Mutex<System>> = Lazy::new(|| Mutex::new(System::new_all()));

pub(crate) async fn get_cpu_ram() -> (Vec<u32>, u32) {
    use sysinfo::CpuExt;
    let mut lock = SYS.lock().await;
    lock.refresh_cpu();
    lock.refresh_memory();

    let cpus: Vec<u32> = lock
        .cpus()
        .iter()
        .map(|cpu| cpu.cpu_usage() as u32) // Always rounds down
        .collect();

    let memory = (lock.used_memory() as f32 / lock.total_memory() as f32) * 100.0;

    //println!("cpu: {:?}, ram: {}", cpus, memory);

    (cpus, memory as u32)
}