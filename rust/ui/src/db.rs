use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use codegraph_rs::query::QueryService;
use codegraph_rs::types::{Edge, Node};

pub struct GraphData {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub unused_no_inbound: HashSet<String>,
    pub unused_unexported: HashSet<String>,
    pub orphan_files: HashSet<String>,
    pub dead_routes: HashSet<String>,
}

pub fn load_graph(project_root: &Path) -> Result<GraphData> {
    let qs = QueryService::open(project_root)?;

    let nodes = qs.get_all_nodes(5_000)?;
    let edges = qs.get_all_edges(50_000)?;

    let unused_no_inbound = qs.get_unused_no_inbound_ids()?.into_iter().collect();
    let unused_unexported = qs.get_unused_unexported_unreferenced_ids()?.into_iter().collect();
    let orphan_files = qs.get_orphan_file_ids()?.into_iter().collect();
    let dead_routes = qs.get_dead_route_ids()?.into_iter().collect();

    Ok(GraphData {
        nodes,
        edges,
        unused_no_inbound,
        unused_unexported,
        orphan_files,
        dead_routes,
    })
}
