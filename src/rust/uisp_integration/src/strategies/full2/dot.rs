use std::fs::write;
use crate::errors::UispIntegrationError;
use crate::strategies::full2::GraphType;

pub fn save_dot_file(graph: &GraphType) -> Result<(), UispIntegrationError> {
    // Save the dot file
    let dot_data = format!("{:?}", petgraph::dot::Dot::with_config(graph, &[petgraph::dot::Config::EdgeNoLabel]));
    let _ = write("graph.dot", dot_data.as_bytes());
    let _ = std::process::Command::new("dot")
        .arg("-Tpng")
        .arg("graph.dot")
        .arg("-o")
        .arg("graph.png")
        .output();
    Ok(())
}