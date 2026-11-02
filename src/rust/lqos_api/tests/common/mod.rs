#![allow(dead_code)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use axum::Router;

pub struct TestEnv {
    /// Owns the temp root: dropping it removes the tree (see [`env`]).
    _temp_dir: tempfile::TempDir,
    pub root_dir: PathBuf,
    pub config_path: PathBuf,
    pub network_json_path: PathBuf,
    pub shaped_devices_path: PathBuf,
}

static TEST_ENV: OnceLock<TestEnv> = OnceLock::new();

/// The repository's `lqos.example`, resolved from the crate root so the path does
/// not depend on the test process's working directory.
pub fn lqos_example_path() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lqos.example"))
}

/// Reads the repository's `lqos.example`.
pub fn lqos_example() -> String {
    let path = lqos_example_path();
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

pub fn env() -> &'static TestEnv {
    TEST_ENV.get_or_init(|| {
        // Keep the temp tree inside the test target directory: the guard stays
        // RAII (no `.keep()`), and a process-lifetime `TestEnv` cannot leak a
        // `/tmp/.tmp*` root either way.
        let temp_dir = tempfile::Builder::new()
            .prefix("lqos-api-test-env-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .expect("create temp dir");
        let root_dir = temp_dir.path().to_path_buf();

        let config_path = root_dir.join("lqos.conf");
        let example = lqos_example();
        let state_dir = root_dir.join("state");
        std::fs::create_dir_all(state_dir.join("topology"))
            .expect("create test topology state dir");
        let example = example
            .replace(
                "lqos_directory = \"/opt/libreqos/src\"",
                &format!("lqos_directory = \"{}\"", root_dir.display()),
            )
            .replace(
                "state_directory = \"/opt/libreqos/state\"",
                &format!("state_directory = \"{}\"", state_dir.display()),
            );
        std::fs::write(&config_path, example).expect("write test lqos.conf");

        unsafe {
            std::env::set_var("LQOS_CONFIG", &config_path);
            std::env::set_var("LQOS_DIRECTORY", &root_dir);
        }

        let network_json_path = root_dir.join("network.json");
        let shaped_devices_path = root_dir.join("ShapedDevices.csv");

        // Create baseline files
        reset_network_json_at(&network_json_path);
        reset_shaped_devices_at(&shaped_devices_path);

        TestEnv {
            _temp_dir: temp_dir,
            root_dir,
            config_path,
            network_json_path,
            shaped_devices_path,
        }
    })
}

pub fn reset_network_json() {
    reset_network_json_at(&env().network_json_path);
}

fn reset_network_json_at(path: &Path) {
    std::fs::write(path, "{}\n").expect("write baseline network.json");
}

pub fn reset_shaped_devices() {
    reset_shaped_devices_at(&env().shaped_devices_path);
}

fn reset_shaped_devices_at(path: &Path) {
    // Must match the header we emit in `shaped_devices_to_csv_string`.
    let header = "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment,sqm\n";
    std::fs::write(path, header).expect("write baseline ShapedDevices.csv");
}

/// The mount's state with the daemon's handles stubbed: a fixed topology
/// section, a fixed CPU/RAM sample and an empty circuit directory. The mount
/// owns none of these, so a test supplies them instead of starting a sampler.
pub fn api_state() -> std::sync::Arc<lqos_api::api::ApiState> {
    static TOPOLOGY: OnceLock<Option<lqos_api::api::CpuTopologySection>> = OnceLock::new();
    TOPOLOGY.get_or_init(|| {
        Some(lqos_api::api::CpuTopologySection::unavailable(
            "test_stub_topology",
        ))
    });
    lqos_api::api::ApiState::new(&TOPOLOGY, system_usage, circuit_names)
}

fn system_usage() -> lqos_api::api::SystemUsageSnapshot {
    lqos_api::api::SystemUsageSnapshot {
        cpu_usage: vec![1, 2, 3, 4],
        cpu_ids: vec![0, 1, 2, 3],
        num_cpus: 4,
        ram_used: 123,
        total_ram: 456,
    }
}

fn circuit_names() -> HashMap<String, String> {
    HashMap::new()
}

/// The API's data surface, exactly as `lqosd` mounts it.
///
/// No authentication and no licence gate live here: the daemon's `x-bearer`
/// verifier sits in front of the mount, and these tests exercise the routes
/// behind it.
pub fn router() -> Router {
    lqos_api::api_routes(api_state())
}
