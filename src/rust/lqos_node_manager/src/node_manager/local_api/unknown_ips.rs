use lqos_bus::{bus_request, BusRequest, BusResponse, UnknownIp};

pub async fn get_unknown_ips() -> Vec<UnknownIp> {
    let Ok(replies) = bus_request(vec![BusRequest::UnknownIps]).await else {
        return Vec::new();
    };

    for reply in replies.into_iter() {
        if let BusResponse::UnknownIps(ips) = reply {
            return ips;
        }
    }

    Vec::new()
}

pub async fn unknown_ips() -> axum::Json<Vec<UnknownIp>> {
    axum::Json(get_unknown_ips().await)
}

pub async fn unknown_ips_csv() -> String {
    let list = get_unknown_ips().await;

    let mut csv = String::new();
    csv.push_str("IP Address,Total Download (bytes),Total Upload (bytes)\n");
    for unknown in list.into_iter() {
        csv.push_str(&format!(
            "{},{},{}\n",
            unknown.ip,
            unknown.total_bytes.down,
            unknown.total_bytes.up
        ));
    }

    csv
}