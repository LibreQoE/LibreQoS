use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct PacketHost {
    pub node_id: String,
    pub node_name: String,
    pub down: Vec<Packets>,
    pub up: Vec<Packets>,
}

#[derive(Serialize, Debug)]
pub struct Packets {
    pub value: f64,
    pub date: String,
    pub l: f64,
    pub u: f64,
}

#[derive(Serialize, Debug)]
pub struct PacketChart {
    pub msg: String,
    pub nodes: Vec<PacketHost>,
}