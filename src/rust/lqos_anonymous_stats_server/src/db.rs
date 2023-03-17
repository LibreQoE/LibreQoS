use std::path::Path;
const DBPATH: &str = "anonymous.sqlite";

const SETUP_QUERY: &str = 
"CREATE TABLE submissions (
    id INTEGER PRIMARY KEY,
    date TEXT,
    node_id TEXT,
    ip_address TEXT,
    git_hash TEXT,
    using_xdp_bridge INTEGER,
    on_a_stick INTEGER,
    total_memory INTEGER,
    available_memory INTEGER,
    kernel_version TEXT,
    distro TEXT,
    usable_cores INTEGER,
    cpu_brand TEXT,
    cpu_vendor TEXT,
    cpu_frequency INTEGER,
    sqm TEXT,
    monitor_mode INTEGER,
    capacity_down INTEGER,
    capacity_up INTEGER,
    generated_pdn_down INTEGER,
    generated_pdn_up INTEGER,
    shaped_device_count INTEGER,
    net_json_len INTEGER
);

CREATE TABLE nics (
    id INTEGER PRIMARY KEY,
    parent INTEGER,
    description TEXT,
    product TEXT,
    vendor TEXT,
    clock TEXT,
    capacity TEXT,
    FOREIGN KEY(parent) 
        REFERENCES submissions (id) 
            ON DELETE CASCADE
            ON UPDATE NO ACTION
);
";

pub fn create_if_not_exist() {
    let path = Path::new(DBPATH);
    if !path.exists() {
        if let Ok(cn) = sqlite::open(DBPATH) {
            let result = cn.execute(SETUP_QUERY);
            if let Err(e) = result {
                log::error!("{e:?}");
                panic!("Failed to create database");
            }
        } else {
            panic!("Unable to create database");
        }
    }
}