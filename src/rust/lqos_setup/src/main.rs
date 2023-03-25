use colored::Colorize;
use default_net::{get_interfaces, interface::InterfaceType, Interface};
use uuid::Uuid;
use std::{fs, path::Path, process::Command};

fn get_available_interfaces() -> Vec<Interface> {
  get_interfaces()
    .iter()
    .filter(|eth| {
      eth.if_type == InterfaceType::Ethernet && !eth.name.starts_with("br")
    })
    .cloned()
    .collect()
}

fn should_build(path: &str) -> bool {
  if Path::new(path).exists() {
    let string = format!("Skipping: {path}");
    println!("{}", string.red());
    println!("{}", "You already have one installed\n".cyan());
    return false;
  }
  true
}

fn list_interfaces(interfaces: &Vec<Interface>) {
  println!("{}", "Available Interfaces".white());
  for i in interfaces {
    let iftype = format!("{:?}", i.if_type);
    println!("{} - {}", i.name.cyan(), iftype.yellow());
  }
}

fn is_valid_interface(interfaces: &[Interface], iface: &str) -> bool {
  interfaces.iter().any(|i| i.name == iface)
}

pub fn read_line() -> String {
  let mut guess = String::new();
  std::io::stdin().read_line(&mut guess).expect("failed to readline");
  guess.trim().to_string()
}

pub fn read_line_as_number() -> u32 {
  loop {
    let str = read_line();
    if let Ok(n) = str::parse::<u32>(&str) {
      return n;
    }
    println!("Could not parse [{str}] as a number. Try again.");
  }
}

const LQOS_CONF: &str = "/etc/lqos.conf";
const ISP_CONF: &str = "/opt/libreqos/src/ispConfig.py";
const NETWORK_JSON: &str = "/opt/libreqos/src/network.json";
const SHAPED_DEVICES: &str = "/opt/libreqos/src/ShapedDevices.csv";
const LQUSERS: &str = "/opt/libreqos/src/lqusers.toml";

fn get_internet_interface(
  interfaces: &Vec<Interface>,
  if_internet: &mut Option<String>,
) {
  if if_internet.is_none() {
    println!("{}", "Which Network Interface faces the INTERNET?".yellow());
    list_interfaces(interfaces);
    loop {
      let iface = read_line();
      if is_valid_interface(interfaces, &iface) {
        *if_internet = Some(iface);
        break;
      } else {
        println!("{}", "Not a valid interface".red());
      }
    }
  }
}

fn get_isp_interface(
  interfaces: &Vec<Interface>,
  if_isp: &mut Option<String>,
) {
  if if_isp.is_none() {
    println!("{}", "Which Network Interface faces the ISP CORE?".yellow());
    list_interfaces(interfaces);
    loop {
      let iface = read_line();
      if is_valid_interface(interfaces, &iface) {
        *if_isp = Some(iface);
        break;
      } else {
        println!("{}", "Not a valid interface".red());
      }
    }
  }
}

fn get_bandwidth(up: bool) -> u32 {
  loop {
    match up {
      true => println!(
        "{}",
        "How much UPLOAD bandwidth do you have? (Mbps, e.g. 1000 = 1 gbit)"
          .yellow()
      ),
      false => println!(
        "{}",
        "How much DOWNLOAD bandwidth do you have? (Mbps, e.g. 1000 = 1 gbit)"
          .yellow()
      ),
    }
    let bandwidth = read_line_as_number();
    if bandwidth > 0 {
      return bandwidth;
    }
  }
}

const ETC_LQOS_CONF: &str = "lqos_directory = '/opt/libreqos/src'
queue_check_period_ms = 1000
node_id = \"{NODE_ID}\"

[tuning]
stop_irq_balance = true
netdev_budget_usecs = 8000
netdev_budget_packets = 300
rx_usecs = 8
tx_usecs = 8
disable_rxvlan = true
disable_txvlan = true
disable_offload = [ \"gso\", \"tso\", \"lro\", \"sg\", \"gro\" ]

[bridge]
use_xdp_bridge = true
interface_mapping = [
       { name = \"{INTERNET}\", redirect_to = \"{ISP}\", scan_vlans = false },
       { name = \"{ISP}\", redirect_to = \"{INTERNET}\", scan_vlans = false }
]
vlan_mapping = []

[usage_stats]
send_anonymous = {ALLOW_ANONYMOUS}
anonymous_server = \"stats.libreqos.io:9125\"
";

fn write_etc_lqos_conf(internet: &str, isp: &str, allow_anonymous: bool) {
  let new_id = Uuid::new_v4().to_string();
  let output =
    ETC_LQOS_CONF.replace("{INTERNET}", internet).replace("{ISP}", isp)
    .replace("{NODE_ID}", &new_id)
    .replace("{ALLOW_ANONYMOUS}", &allow_anonymous.to_string());
  fs::write(LQOS_CONF, output).expect("Unable to write file");
}

pub fn write_isp_config_py(
  dir: &str,
  download: u32,
  upload: u32,
  lan: &str,
  internet: &str,
) {
  // Copy ispConfig.example.py to ispConfig.py
  let orig = format!("{dir}ispConfig.example.py");
  let dest = format!("{dir}ispConfig.py");
  std::fs::copy(orig, &dest).unwrap();

  let config_file = std::fs::read_to_string(&dest).unwrap();
  let mut new_config_file = String::new();
  config_file.split('\n').for_each(|line| {
    if line.starts_with('#') {
      new_config_file += line;
      new_config_file += "\n";
    } else if line.contains("upstreamBandwidthCapacityDownloadMbps") {
      new_config_file +=
        &format!("upstreamBandwidthCapacityDownloadMbps = {download}\n");
    } else if line.contains("upstreamBandwidthCapacityUploadMbps") {
      new_config_file +=
        &format!("upstreamBandwidthCapacityUploadMbps = {upload}\n");
    } else if line.contains("interfaceA") {
      new_config_file += &format!("interfaceA = \"{lan}\"\n");
    } else if line.contains("interfaceB") {
      new_config_file += &format!("interfaceB = \"{internet}\"\n");
    } else if line.contains("generatedPNDownloadMbps") {
      new_config_file += &format!("generatedPNDownloadMbps = {download}\n");
    } else if line.contains("generatedPNUploadMbps") {
      new_config_file += &format!("generatedPNUploadMbps = {upload}\n");
    } else {
      new_config_file += line;
      new_config_file += "\n";
    }
  });
  std::fs::write(&dest, new_config_file).unwrap();
}

fn write_network_json() {
  let output = "{}\n";
  fs::write(NETWORK_JSON, output).expect("Unable to write file");
}

const EMPTY_SHAPED_DEVICES: &str = "# This is a comment
Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment
# This is another comment
\"9999\",\"968 Circle St., Gurnee, IL 60031\",1,Device 1,AP_7,,\"100.64.1.2, 100.64.0.14\",,25,5,500,500,";

fn write_shaped_devices() {
  fs::write(SHAPED_DEVICES, EMPTY_SHAPED_DEVICES)
    .expect("Unable to write file");
}

fn anonymous() -> bool {
  println!("{}", "Help Improve LibreQoS with Anonymous Statistics?".yellow());
  println!("{}", "We'd really appreciate it if you'd allow anonymous statistics".green());
  println!("{}", "to be sent to our way. They help us focus our develpoment,".green());
  println!("{}", "and let us know that you're out there!".green());
  loop {
    println!("{}", "Reply YES or NO".cyan());
    let input = read_line().trim().to_uppercase();
    if input == "YES" {
      return true;
    } else if input == "NO" {
      return false;
    }
  }
}

fn main() {
  println!("{:^80}", "LibreQoS 1.4 Setup Assistant".yellow().on_blue());
  println!();
  let interfaces = get_available_interfaces();
  let mut if_internet: Option<String> = None;
  let mut if_isp: Option<String> = None;
  if should_build(LQOS_CONF) {
    println!(
      "{}{}",
      LQOS_CONF.cyan(),
      "does not exist, building one.".white()
    );
    get_internet_interface(&interfaces, &mut if_internet);
    get_isp_interface(&interfaces, &mut if_isp);
    let allow_anonymous = anonymous();
    if let (Some(internet), Some(isp)) = (&if_internet, &if_isp) {
      write_etc_lqos_conf(internet, isp, allow_anonymous);
    }
  }

  if should_build(ISP_CONF) {
    println!("{}{}", ISP_CONF.cyan(), "does not exist, building one.".white());
    get_internet_interface(&interfaces, &mut if_internet);
    get_isp_interface(&interfaces, &mut if_isp);
    let upload = get_bandwidth(true);
    let download = get_bandwidth(false);
    if let (Some(internet), Some(isp)) = (&if_internet, &if_isp) {
      write_isp_config_py(
        "/opt/libreqos/src/",
        download,
        upload,
        isp,
        internet,
      )
    }
  }

  if should_build(NETWORK_JSON) {
    println!(
      "{}{}",
      NETWORK_JSON.cyan(),
      "does not exist, making a simple flat network.".white()
    );
    write_network_json();
  }
  if should_build(SHAPED_DEVICES) {
    println!(
      "{}{}",
      SHAPED_DEVICES.cyan(),
      "does not exist, making an empty one.".white()
    );
    println!("{}", "Don't forget to add some users!".magenta());
    write_shaped_devices();
  }
  if should_build(LQUSERS) {
    println!("Enter a username for the web manager:");
    let user = read_line();
    println!("Enter a password for the web manager:");
    let password = read_line();
    Command::new("/opt/libreqos/src/bin/lqusers")
      .args([
        "add",
        "--username",
        &user,
        "--role",
        "admin",
        "--password",
        &password,
      ])
      .output()
      .unwrap();
  }
}
