use anyhow::{Error, Result};
use clap::{Parser, Subcommand};
use lqos_bus::{BusRequest, BusResponse, IpMapping, TcHandle, bus_request};
use lqos_utils::hex_string::read_hex_string;
use std::process::exit;

#[derive(Parser)]
#[command()]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add an IP Address (v4 or v6) to the XDP/TC mapping system.
    Add {
        /// IP Address (v4 or v6) to add
        #[arg(long)]
        ip: String,

        /// TC Class ID (handle) to connect
        #[arg(long)]
        classid: String,

        /// CPU id to connect
        #[arg(long)]
        cpu: String,

        /// Circuit ID (raw string). Hashed before being stored.
        #[arg(long)]
        circuit_id: Option<String>,

        /// Device ID (raw string). Hashed before being stored.
        #[arg(long)]
        device_id: Option<String>,

        /// Add "--upload 1" if you are using on-a-stick and need to map upload separately
        #[arg(long)]
        upload: Option<String>,
    },
    /// Remove an IP address (v4 or v6) from the XDP/TC mapping system.
    Del {
        /// IP Address (v4 or v6) to remove
        ip: String,

        /// Add "--upload 1" if you are using on-a-stick and need to map upload separately
        #[arg(long)]
        upload: Option<String>,
    },
    /// Clear all mapped IPs.
    Clear,
    /// List all mapped IPs.
    List,
    /// Flushes the Hot Cache (to be used after when you are done making changes).
    Flush,
}

async fn talk_to_server(command: BusRequest) -> Result<()> {
    let responses = bus_request(vec![command]).await?;
    match &responses[0] {
        BusResponse::Ack => {
            println!("Success");
            Ok(())
        }
        BusResponse::Fail(err) => Err(Error::msg(err.clone())),
        BusResponse::MappedIps(ips) => {
            print_ips(ips);
            Ok(())
        }
        _ => Err(Error::msg("Command execution failed")),
    }
}

fn print_ips(ips: &[IpMapping]) {
    println!("\nMapped IP Addresses:");
    println!("--------------------------------------------------------------------");
    for ip in ips.iter() {
        let ip_formatted = if ip.ip_address.contains(':') {
            format!("{}/{}", ip.ip_address, ip.prefix_length)
        } else {
            format!("{}/{}", ip.ip_address, ip.prefix_length - 96)
        };
        println!(
            "{:<45} CPU: {:<4} TC: {} CIRCUIT: {} DEVICE: {}",
            ip_formatted,
            ip.cpu,
            ip.tc_handle.to_string(),
            ip.circuit_id as i64,
            ip.device_id as i64
        );
    }
    println!();
}

fn parse_add_ip(
    ip: &str,
    classid: &str,
    cpu: &str,
    upload: &Option<String>,
    circuit_id: &Option<String>,
    device_id: &Option<String>,
) -> Result<BusRequest> {
    //if ip.parse::<IpAddr>().is_err() {
    //    return Err(Error::msg(format!("Unable to parse IP address: {ip}")));
    //}
    if !classid.contains(':') {
        return Err(Error::msg(format!(
            "Class id must be in the format (major):(minor), e.g. 1:12. Provided string: {classid}"
        )));
    }
    let circuit_id = circuit_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| lqos_utils::hash_to_i64(s) as u64)
        .unwrap_or(0);
    let device_id = device_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| lqos_utils::hash_to_i64(s) as u64)
        .unwrap_or(0);
    Ok(BusRequest::MapIpToFlow {
        ip_address: ip.to_string(),
        tc_handle: TcHandle::from_string(classid)?,
        cpu: read_hex_string(cpu)?, // Force HEX representation
        circuit_id,
        device_id,
        upload: upload.is_some(),
    })
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<()> {
    let cli = Args::parse();

    match cli.command {
        Some(Commands::Add {
            ip,
            classid,
            cpu,
            circuit_id,
            device_id,
            upload,
        }) => {
            talk_to_server(parse_add_ip(&ip, &classid, &cpu, &upload, &circuit_id, &device_id)?)
                .await?;
        }
        Some(Commands::Del { ip, upload }) => {
            talk_to_server(BusRequest::DelIpFlow {
                ip_address: ip.to_string(),
                upload: upload.is_some(),
            })
            .await?
        }
        Some(Commands::Clear) => talk_to_server(BusRequest::ClearIpFlow).await?,
        Some(Commands::List) => talk_to_server(BusRequest::ListIpFlow).await?,
        Some(Commands::Flush) => talk_to_server(BusRequest::ClearHotCache).await?,
        None => {
            println!("Run with --help to see instructions");
            exit(0);
        }
    }

    Ok(())
}
