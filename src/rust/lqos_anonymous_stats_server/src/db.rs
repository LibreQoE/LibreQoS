use std::{path::Path, sync::atomic::AtomicI64, time::SystemTime};
use lqos_bus::anonymous::AnonymousUsageV1;
use sqlite::Value;
use chrono::prelude::{DateTime, Utc};
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

static SUBMISSION_ID: AtomicI64 = AtomicI64::new(0);

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

pub fn check_id() {
    log::info!("Checking primary keys");
    if let Ok(cn) = sqlite::open(DBPATH) {
        cn.iterate("SELECT MAX(id) as id FROM submissions;", |pairs| {
            for &(name, value) in pairs.iter() {
                if name == "id" {
                    if let Some(val) = value {
                        if let Ok(n) = val.parse::<i64>() {
                            log::info!("Last id: {n}");
                            SUBMISSION_ID.store(n+1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
            }
            true
        }).unwrap();
    } else {
        panic!("Unable to connect to database");
    }
}

const INSERT_STATS: &str =
"INSERT INTO submissions VALUES (
    :id, :date, :node_id, :ip_address, :git_hash, :using_xdp_bridge, :on_a_stick,
    :total_memory, :available_memory, :kernel_version, :distro, :usable_cores,
    :cpu_brand, :cpu_vendor, :cpu_frequency, :sqm, :monitor_mode, :capcity_down,
    :capacity_up, :genereated_pdn_down, :generated_pdn_up, :shaped_device_count,
    :net_json_len
);";

const INSERT_NIC: &str =
"INSERT INTO nics 
(parent, description, product, vendor, clock, capacity)
VALUES (
    :parent, :description, :product, :vendor, :clock, :capacity
);";

fn iso8601(st: std::time::SystemTime) -> String {
    let dt: DateTime<Utc> = st.into();
    format!("{}", dt.format("%+"))
    // formats like "2001-07-08T00:34:60.026490+09:30"
}

fn bool_to_n(x: bool) -> i64 {
    if x {
        1
    } else {
        0
    }
}

pub fn insert_stats_dump(stats: &AnonymousUsageV1, ip: &str) -> anyhow::Result<()> {
    let date = iso8601(SystemTime::now());
    let new_id = SUBMISSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let cn = sqlite::open(DBPATH)?;
    let mut statement = cn.prepare(INSERT_STATS)?;
    statement.bind_iter::<_, (_, Value)>([
        (":id", new_id.into()),
        (":date", date.into()),
        (":node_id", stats.node_id.clone().into()),
        (":ip_address", ip.into()),
        (":git_hash", stats.git_hash.clone().into()),
        (":using_xdp_bridge", bool_to_n(stats.using_xdp_bridge).into()),
        (":on_a_stick", bool_to_n(stats.on_a_stick).into()),
        (":total_memory", (stats.total_memory as i64).into()),
        (":available_memory", (stats.available_memory as i64).into()),
        (":kernel_version", stats.kernel_version.clone().into()),
        (":distro", stats.distro.clone().into()),
        (":usable_cores", (stats.usable_cores as i64).into()),
        (":cpu_brand", stats.cpu_brand.clone().into()),
        (":cpu_vendor", stats.cpu_vendor.clone().into()),
        (":cpu_frequency", (stats.cpu_frequency as i64).into()),
        (":sqm", stats.sqm.clone().into()),
        (":monitor_mode", bool_to_n(stats.monitor_mode).into()),
        (":capcity_down", (stats.total_capacity.0 as i64).into()),
        (":capacity_up", (stats.total_capacity.1 as i64).into()),
        (":genereated_pdn_down", (stats.generated_pdn_capacity.0 as i64).into()),
        (":generated_pdn_up", (stats.generated_pdn_capacity.1 as i64).into()),
        (":shaped_device_count", (stats.shaped_device_count as i64).into()),
        (":net_json_len", (stats.net_json_len as i64).into()),
    ])?;
    statement.next()?;

    for nic in stats.nics.iter() {
        let mut statement = cn.prepare(INSERT_NIC)?;
        statement.bind_iter::<_, (_, Value)>([
            (":parent", new_id.into()),
            (":description", nic.description.clone().into()),
            (":product", nic.product.clone().into()),
            (":vendor", nic.vendor.clone().into()),
            (":clock", nic.clock.clone().into()),
            (":capacity", nic.capacity.clone().into()),
        ])?;
        statement.next()?;
    }

    log::info!("Submitted");
    Ok(())
}

// Not a great idea, this is for test data
pub fn dump_all_to_string() -> anyhow::Result<String> {
    let mut result = String::new();
    let cn = sqlite::open(DBPATH)?;
    cn.iterate("SELECT * FROM submissions;", |pairs| {
        for &(name, value) in pairs.iter() {
            result += &format!(";{name}={value:?}");
        }
        result += "\n";
        true
    }).unwrap();
    Ok(result)
}