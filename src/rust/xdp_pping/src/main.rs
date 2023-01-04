use anyhow::Result;
use lqos_bus::{BUS_BIND_ADDRESS, BusSession, BusRequest, encode_request, decode_response, BusResponse};
use tokio::{net::TcpStream, io::{AsyncReadExt, AsyncWriteExt}};

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<()> {
    let mut stream = TcpStream::connect(BUS_BIND_ADDRESS).await?;
    let test = BusSession {
        auth_cookie: 1234,
        requests: vec![BusRequest::XdpPping],
    };
    let msg = encode_request(&test)?;
    stream.write(&msg).await?;
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await.unwrap();
    let reply = decode_response(&buf)?;
    for resp in reply.responses.iter() {
        match resp {
            BusResponse::XdpPping(lines) => {
                println!("[");
                for line in lines.iter() {
                    println!("{{\"tc\":\"{}\", \"avg\": {}, \"min\": {}, \"max\": {}, \"median\": {}, \"samples\": {}}}",
                        line.tc,
                        line.avg,
                        line.min,
                        line.max,
                        line.median,
                        line.samples,
                    );
                }
                println!("{{}}]");
            }
            _ => {}
        }
    }
    Ok(())
}