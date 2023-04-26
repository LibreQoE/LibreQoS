use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct Rtt {
    pub value: f64,
    pub date: String,
    pub l: f64,
    pub u: f64,
}

#[derive(Serialize, Debug)]
pub struct RttHost {
    pub node_id: String,
    pub node_name: String,
    pub rtt: Vec<Rtt>,
}

#[derive(Serialize, Debug)]
pub struct RttChart {
    pub msg: String,
    pub nodes: Vec<RttHost>,
    pub histogram: Vec<u32>,
}