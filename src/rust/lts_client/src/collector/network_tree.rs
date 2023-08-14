use lqos_config::NetworkJsonNode;

#[derive(Debug, Clone)]
pub struct NetworkTreeEntry {
    pub name: String,
    pub max_throughput: (u32, u32),
    pub current_throughput: (u32, u32),
    pub rtts: (u16, u16, u16),
    pub parents: Vec<usize>,
    pub immediate_parent: Option<usize>,
    pub node_type: Option<String>,
}

impl From<&NetworkJsonNode> for NetworkTreeEntry {
    fn from(value: &NetworkJsonNode) -> Self {
        let mut max = 0;
        let mut min = if value.rtts.is_empty() {
            0
        } else {
            u16::MAX
        };
        let mut sum = 0;
        for n in value.rtts.iter() {
            let n = *n;
            sum += n;
            if n < min { min = n; }
            if n > max { max = n; }
        }
        let avg = sum.checked_div(value.rtts.len() as u16).unwrap_or(0);

        Self {
            name: value.name.clone(),
            max_throughput: value.max_throughput,
            parents: value.parents.clone(),
            immediate_parent: value.immediate_parent,
            current_throughput: (
                value.current_throughput.0.load(std::sync::atomic::Ordering::Relaxed) as u32,
                value.current_throughput.1.load(std::sync::atomic::Ordering::Relaxed) as u32,
            ),
            node_type: value.node_type.clone(),
            rtts: (min, max, avg),
        }
    }
}
