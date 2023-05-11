use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct ThroughputHost {
    pub node_id: String,
    pub node_name: String,
    pub down: Vec<Throughput>,
    pub up: Vec<Throughput>,
}

#[derive(Serialize, Debug, Clone)]
pub struct Throughput {
    pub value: f64,
    pub date: String,
    pub l: f64,
    pub u: f64,
}

#[derive(Serialize, Debug)]
pub struct ThroughputChart {
    pub msg: String,
    pub nodes: Vec<ThroughputHost>,
}