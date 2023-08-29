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
            u64::MAX
        };
        let mut sum: u64 = 0;
        let mut count = 0;
        for n in value.rtts.iter() {
            let n = *n as u64;
            if n > 0 {
                sum += n;
                if n < min { min = n; }
                if n > max { max = n; }
                count += 1;
            }
        }
        let avg = sum.checked_div(count).unwrap_or(0);

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
            rtts: (min as u16, max as u16, avg as u16),
        }
    }
}
