use anyhow::Result;
use lqos_bus::{
    BusRequest, BusResponse, bus_request,
};

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<()> {
    for resp in bus_request(vec![BusRequest::XdpPping]).await? {
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
