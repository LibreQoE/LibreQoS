use lqos_config::ConfigShapedDevices;
use lqos_utils::file_watcher::FileWatcher;

use super::Task;

fn load_shaped_devices() -> ConfigShapedDevices {
}

fn watch_for_shaped_devices_changing() -> Result<(), ()> {
    let watch_path = ConfigShapedDevices::path();
    let watch_path = watch_path.unwrap();
    let mut watcher = FileWatcher::new("ShapedDevices.csv", watch_path);
    watcher.set_file_exists_callback(load_shaped_devices);
    watcher.set_file_created_callback(load_shaped_devices);
    watcher.set_file_changed_callback(load_shaped_devices);
    loop {
        let result = watcher.watch();
        tracing::debug!("ShapedDevices watcher returned: {:?}", result);
    }
}

pub struct Devices {
    shaped: ConfigShapedDevices
}

impl Devices {
    async fn get(&self) -> Self {
        Devices {
            shaped: ConfigShapedDevices::load().unwrap()
        }
    }
}

impl Task for Devices {
    fn execute(&self) -> TaskResult {
        self.get()
    }

    fn key(&self) -> String {
        String::from("DEVICES")
    }

    fn cacheable(&self) -> bool { true }
}