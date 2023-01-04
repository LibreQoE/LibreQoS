use anyhow::{Error, Result};
use clap::{Parser, Subcommand};
use lqos_bus::{
    decode_response, encode_request, BusRequest, BusResponse, BusSession, IpMapping,
    BUS_BIND_ADDRESS, TcHandle,
};
use std::process::exit;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

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

        /// Add "--upload 1" if you are using on-a-stick and need to map upload separately
        #[arg(long)]
        upload: Option<String>
    },
    /// Remove an IP address (v4 or v6) from the XDP/TC mapping system.
    Del {
        /// IP Address (v4 or v6) to remove
        ip: String,

        /// Add "--upload 1" if you are using on-a-stick and need to map upload separately
        #[arg(long)]
        upload: Option<String>
    },
    /// Clear all mapped IPs.
    Clear,
    /// List all mapped IPs.
    List,
}

async fn talk_to_server(command: BusRequest) -> Result<()> {
    let mut stream = TcpStream::connect(BUS_BIND_ADDRESS).await?;
    let test = BusSession {
        auth_cookie: 1234,
        requests: vec![command],
    };
    let msg = encode_request(&test)?;
    stream.write(&msg).await?;
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await.unwrap();
    let reply = decode_response(&buf)?;
    match &reply.responses[0] {
        BusResponse::Ack => {
            println!("Success");
            Ok(())
        }
        BusResponse::Fail(err) => Err(Error::msg(err.clone())),
        BusResponse::MappedIps(ips) => {
            print_ips(&ips);
            Ok(())
        }
        _ => Err(Error::msg("Command execution failed")),
    }
}

fn print_ips(ips: &[IpMapping]) {
    println!("\nMapped IP Addresses:");
    println!("--------------------------------------------------------------------");
    for ip in ips.iter() {
        let ip_formatted = if ip.ip_address.contains(":") {
            format!("{}/{}", ip.ip_address, ip.prefix_length)
        } else {
            format!("{}/{}", ip.ip_address, ip.prefix_length - 96)
        };
        println!(
            "{:<45} CPU: {:<4} TC: {}",
            ip_formatted, ip.cpu, ip.tc_handle.to_string()
        );
    }
    println!("");
}

fn parse_add_ip(ip: &str, classid: &str, cpu: &str, upload: &Option<String>) -> Result<BusRequest> {
    //if ip.parse::<IpAddr>().is_err() {
    //    return Err(Error::msg(format!("Unable to parse IP address: {ip}")));
    //}
    if !classid.contains(":") {
        return Err(Error::msg(
            "Class id must be in the format (major):(minor), e.g. 1:12",
        ));
    }
    Ok(BusRequest::MapIpToFlow {
        ip_address: ip.to_string(),
        tc_handle: TcHandle::from_string(classid)?,
        cpu: u32::from_str_radix(&cpu.replace("0x", ""), 16)?, // Force HEX representation
        upload: upload.is_some(),
    })
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<()> {
    let cli = Args::parse();

    match cli.command {
        Some(Commands::Add { ip, classid, cpu, upload }) => {
            talk_to_server(parse_add_ip(&ip, &classid, &cpu, &upload)?).await?;
        }
        Some(Commands::Del { ip , upload }) => {
            talk_to_server(BusRequest::DelIpFlow {
                ip_address: ip.to_string(),
                upload: upload.is_some(),
            })
            .await?
        }
        Some(Commands::Clear) => talk_to_server(BusRequest::ClearIpFlow).await?,
        Some(Commands::List) => talk_to_server(BusRequest::ListIpFlow).await?,
        None => {
            println!("Run with --help to see instructions");
            exit(0);
        }
    }

    Ok(())
}
