use anyhow::Result;
use lqos_config::Config;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use tracing::{debug, info, warn};

static STICK_OFFSET: AtomicU32 = AtomicU32::new(0);

pub(crate) fn stick_offset() -> u32 {
    STICK_OFFSET.load(Ordering::Relaxed)
}

pub(crate) fn recompute_stick_offset(config: &Config) -> Result<u32> {
    let offset = compute_stick_offset(config)?;
    STICK_OFFSET.store(offset, Ordering::Relaxed);
    Ok(offset)
}

fn compute_stick_offset(config: &Config) -> Result<u32> {
    if !config.on_a_stick_mode() {
        return Ok(0);
    }

    let interface = config.internet_interface();

    let queues_available = if let Some(override_available_queues) = config.queues.override_available_queues {
        info!(
            "On-a-stick: overriding available queues to {}",
            override_available_queues
        );
        override_available_queues
    } else {
        count_tx_queues(&interface)?
    };

    let cpu_count = lqos_sys::num_possible_cpus().map_err(|e| anyhow::anyhow!("{e:?}"))?;

    let queues_available = u32::min(queues_available, cpu_count);
    let queues_per_direction = queues_available / 2;
    if queues_per_direction == 0 {
        warn!(
            "On-a-stick: computed 0 queues per direction (queues_available={}, cpu_count={})",
            queues_available, cpu_count
        );
    } else {
        debug!(
            "On-a-stick: queues_available={}, cpu_count={}, queues_per_direction={}, stick_offset={}",
            queues_available, cpu_count, queues_per_direction, queues_per_direction
        );
    }

    Ok(queues_per_direction)
}

fn count_tx_queues(interface: &str) -> Result<u32> {
    let path = format!("/sys/class/net/{interface}/queues/");
    let sys_path = Path::new(&path);
    if !sys_path.exists() {
        return Err(anyhow::anyhow!(
            "/sys/class/net/{interface}/queues/ does not exist. Does this card only support one queue (not supported)?"
        ));
    }

    let mut tx_queues = 0u32;
    for path in std::fs::read_dir(sys_path)? {
        if let Ok(path) = &path {
            if path.path().is_dir() {
                if let Some(filename) = path.path().file_name().and_then(|s| s.to_str()) {
                    if filename.starts_with("tx-") {
                        tx_queues += 1;
                    }
                }
            }
        }
    }

    if tx_queues == 0 {
        Err(anyhow::anyhow!(
            "Interface {} does not have any TX queues.",
            interface
        ))
    } else {
        Ok(tx_queues)
    }
}

