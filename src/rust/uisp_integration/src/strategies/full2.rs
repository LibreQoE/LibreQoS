use std::sync::Arc;
use petgraph::data::Build;
use tracing::{error, warn};
use lqos_config::Config;
use uisp::{DataLinkSite, Site};
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::ap_site::GraphMapping;
use crate::strategies::common::UispData;
use crate::uisp_types::UispSiteType;

pub async fn build_full_network_v2(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Fetch the data
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;

    // Report on obvious UISP errors that should be fixed
    let _trouble = crate::strategies::ap_site::find_troublesome_sites(&uisp_data)
        .map_err(|e|{
            error!("Error finding troublesome sites");
            error!("{e:?}");
            UispIntegrationError::UnknownSiteType
        })?;

    // Find the clients
    let ap_mappings = uisp_data.map_clients_to_aps();

    // Make AP Layer entries
    let access_points = crate::strategies::ap_site::get_ap_layer(&ap_mappings);

    // Site mappings
    let sites = crate::strategies::ap_site::map_sites_above_aps(&uisp_data, ap_mappings, access_points);

    // Insert the root
    let mut root = crate::strategies::ap_site::Layer {
        id: GraphMapping::Root,
        children: sites.values().cloned().collect(),
    };

    // Now transform this into a Petgraph graph
    let mut graph = petgraph::Graph::<GraphMapping, ()>::new();
    root.walk_to_pet_graph(&mut graph, None);

    // Then add other sites
    for site in uisp_data.sites_raw.iter() {
        // Skip clients
        if site.is_client_site() {
            continue;
        }
        // Find the node index of an entry in the graph if there is one
        let mut node_index = graph
            .node_indices()
            .find(|i| graph[*i] == GraphMapping::SiteByName(site.name_or_blank()));

        // If it already exists, then we're interested in finding links between it and other sites
        if let Some(node_index) = node_index {
            // Do nothing
        } else {
            // If it doesn't exist, then we need to add it
            graph.add_node(GraphMapping::SiteByName(site.name_or_blank()));
        }
    }

    // Now iterate all the links!
    for link in uisp_data.data_links_raw.iter() {
        if let Some(from_site) = &link.from.site {
            if let Some(to_site) = &link.to.site {
                add_link_if_new(from_site, to_site, &mut graph, &uisp_data.sites_raw);
            }
        }
    }

    // Find the root
    let root_site_name = config.uisp_integration.site.clone();
    let Some(root_idx) = graph.node_indices().find(|n| {
        if let GraphMapping::SiteByName(name) = &graph[*n] {
            *name == root_site_name
        } else {
            false
        }
    }) else {
        error!("Unable to locate the root note, {}", root_site_name);
        return Err(UispIntegrationError::NoRootSite);
    };

    // Working up to adding the shaper to the detected (named) root
    let mut orphans = None;
    for node in graph.node_indices() {
        if node == root_idx {
            continue;
        }
        let a_star_run = petgraph::algo::astar(&graph, root_idx, |n| n == node, |e| 1, |_| 0);

        let all_paths = petgraph::algo::all_simple_paths::<Vec<_>, _>(&graph, node, root_idx, 0, None)
            .collect::<Vec<_>>();

        if a_star_run.is_none() {
            warn!("No path is detected from {:?} to {}", graph[node], root_site_name);
            if orphans.is_none() {
                orphans = Some(graph.add_node(GraphMapping::GeneratedSiteByName("Orphans".to_string())));
                graph.add_edge(node, root_idx, ());
            }
            if let Some(orphans) = orphans {
                graph.add_edge(node, orphans, ());
            }
        } else {
            //println!("Path detected from {:?} to {}", graph[node], root_site_name);
            if all_paths.len() > 1 {
                println!("Multiple paths detected from {:?} to {}", graph[node], root_site_name);
            }
        }
    }

    // Do topology tracing to find downstream routes and flag alternatives

    // Save the dot file
    //println!("{:?}", petgraph::dot::Dot::with_config(&graph, &[petgraph::dot::Config::EdgeNoLabel]));

    Ok(())
}

fn add_link_if_new(
    from_site: &DataLinkSite,
    to_site: &DataLinkSite,
    graph: &mut petgraph::Graph<GraphMapping, ()>,
    sites_raw: &[Site],
) {
    if from_site.identification.id == to_site.identification.id {
        // If the sites are the same, we don't need to add an edge
        return;
    }
    let Some(site_a) = sites_raw.iter().find(|s| s.id == from_site.identification.id) else {
        return;
    };
    if site_a.is_client_site() {
        return;
    }
    let Some(node_a) = graph.node_indices().find(|n| graph[*n] == GraphMapping::SiteByName(site_a.name_or_blank())) else {
        return;
    };
    let Some(site_b) = sites_raw.iter().find(|s| s.id == to_site.identification.id) else {
        return;
    };
    if site_b.is_client_site() {
        return;
    }
    let Some(node_b) = graph.node_indices().find(|n| graph[*n] == GraphMapping::SiteByName(site_b.name_or_blank())) else {
        return;
    };


    if graph.contains_edge(node_a, node_b) {
        // If the edge already exists, we don't need to add it
        return;
    }
    if graph.contains_edge(node_b, node_a) {
        // If the edge already exists in the opposite direction, we don't need to add it
        return;
    }

    // Add the edge
    //println!("Adding edge from {:?} to {:?}", graph[node_a], graph[node_b]);
    graph.add_edge(node_a, node_b, ());
}

impl crate::strategies::ap_site::Layer {
    fn walk_to_pet_graph(
        &self,
        graph: &mut petgraph::Graph<GraphMapping, ()>,
        parent: Option<petgraph::graph::NodeIndex>,
    ) {
        // Add the current node
        if let GraphMapping::ClientById(..) = self.id {
            // If this is a client, we don't want to add it to the graph
            return;
        }
        let node_index = if let GraphMapping::Root = self.id {
            None
        } else {
            let node_index = graph.add_node(self.id.clone());
            if let Some(parent_index) = parent {
                graph.add_edge(parent_index, node_index, ());
            }
            Some(node_index)
        };

        // Recursively walk to children
        for child in &self.children {
            child.walk_to_pet_graph(graph, node_index);
        }
    }
}