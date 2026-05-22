use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use anyhow::{Result, bail};

use crate::query::QueryService;
use crate::types::{Context, EdgeKind, Node, NodeEdgeRef, NodeKind, Subgraph};

pub struct GraphService<'a> {
    queries: &'a QueryService,
}

impl<'a> GraphService<'a> {
    pub fn new(queries: &'a QueryService) -> Self {
        Self { queries }
    }

    pub fn get_ancestors(&self, node_id: &str) -> Result<Vec<Node>> {
        let mut ancestors = Vec::new();
        let mut visited = HashSet::new();
        let mut current_id = node_id.to_string();

        loop {
            if !visited.insert(current_id.clone()) {
                break;
            }

            let containing_edges = self
                .queries
                .get_incoming_edges(&current_id, Some(&[EdgeKind::Contains]))?;
            let Some(first_edge) = containing_edges.first() else {
                break;
            };

            let Some(parent_node) = self.queries.get_node_by_id(&first_edge.source)? else {
                break;
            };
            current_id = parent_node.id.clone();
            ancestors.push(parent_node);
        }

        Ok(ancestors)
    }

    pub fn get_children(&self, node_id: &str) -> Result<Vec<Node>> {
        let contains_edges = self
            .queries
            .get_outgoing_edges(node_id, Some(&[EdgeKind::Contains]))?;
        let mut children = Vec::new();
        for edge in contains_edges {
            if let Some(child) = self.queries.get_node_by_id(&edge.target)? {
                children.push(child);
            }
        }
        Ok(children)
    }

    pub fn get_callers(&self, node_id: &str, max_depth: usize) -> Result<Vec<NodeEdgeRef>> {
        self.collect_related(
            node_id,
            max_depth,
            true,
            &[EdgeKind::Calls, EdgeKind::References, EdgeKind::Imports],
        )
    }

    pub fn get_callees(&self, node_id: &str, max_depth: usize) -> Result<Vec<NodeEdgeRef>> {
        self.collect_related(
            node_id,
            max_depth,
            false,
            &[EdgeKind::Calls, EdgeKind::References, EdgeKind::Imports],
        )
    }

    pub fn get_context(&self, node_id: &str) -> Result<Context> {
        let Some(focal) = self.queries.get_node_by_id(node_id)? else {
            bail!("Node not found: {node_id}");
        };

        let ancestors = self.get_ancestors(node_id)?;
        let children = self.get_children(node_id)?;

        let mut incoming_refs = Vec::new();
        for edge in self.queries.get_incoming_edges(node_id, None)? {
            if edge.kind == EdgeKind::Contains {
                continue;
            }
            if let Some(node) = self.queries.get_node_by_id(&edge.source)? {
                incoming_refs.push(NodeEdgeRef { node, edge });
            }
        }

        let mut outgoing_refs = Vec::new();
        for edge in self.queries.get_outgoing_edges(node_id, None)? {
            if edge.kind == EdgeKind::Contains {
                continue;
            }
            if let Some(node) = self.queries.get_node_by_id(&edge.target)? {
                outgoing_refs.push(NodeEdgeRef { node, edge });
            }
        }

        let mut types = Vec::new();
        let mut seen_type_ids = HashSet::new();
        for kind in [EdgeKind::TypeOf, EdgeKind::Returns] {
            for edge in self.queries.get_outgoing_edges(node_id, Some(&[kind]))? {
                if let Some(node) = self.queries.get_node_by_id(&edge.target)? {
                    if seen_type_ids.insert(node.id.clone()) {
                        types.push(node);
                    }
                }
            }
        }

        let mut imports = Vec::new();
        if let Some(file_node) = ancestors.iter().find(|node| node.kind == NodeKind::File) {
            for edge in self
                .queries
                .get_outgoing_edges(&file_node.id, Some(&[EdgeKind::Imports]))?
            {
                if let Some(node) = self.queries.get_node_by_id(&edge.target)? {
                    imports.push(node);
                }
            }
        }

        Ok(Context {
            focal,
            ancestors,
            children,
            incoming_refs,
            outgoing_refs,
            types,
            imports,
        })
    }

    pub fn get_file_dependencies(&self, file_path: &str) -> Result<Vec<String>> {
        let nodes = self.queries.get_nodes_by_file(file_path)?;
        let Some(file_node) = nodes.iter().find(|node| node.kind == NodeKind::File) else {
            return Ok(Vec::new());
        };

        let mut dependencies = BTreeSet::new();
        for edge in self
            .queries
            .get_outgoing_edges(&file_node.id, Some(&[EdgeKind::Imports]))?
        {
            if let Some(node) = self.queries.get_node_by_id(&edge.target)? {
                if node.file_path != file_path {
                    dependencies.insert(node.file_path);
                }
            }
        }

        Ok(dependencies.into_iter().collect())
    }

    pub fn get_file_dependents(&self, file_path: &str) -> Result<Vec<String>> {
        let nodes = self.queries.get_nodes_by_file(file_path)?;
        let mut dependents = BTreeSet::new();

        if let Some(file_node) = nodes.iter().find(|node| node.kind == NodeKind::File) {
            for edge in self
                .queries
                .get_incoming_edges(&file_node.id, Some(&[EdgeKind::Imports]))?
            {
                if let Some(node) = self.queries.get_node_by_id(&edge.source)? {
                    if node.file_path != file_path {
                        dependents.insert(node.file_path);
                    }
                }
            }
        }

        for node in &nodes {
            if node.is_exported.unwrap_or(false) {
                for edge in self
                    .queries
                    .get_incoming_edges(&node.id, Some(&[EdgeKind::Imports]))?
                {
                    if let Some(source_node) = self.queries.get_node_by_id(&edge.source)? {
                        if source_node.file_path != file_path {
                            dependents.insert(source_node.file_path);
                        }
                    }
                }
            }
        }

        Ok(dependents.into_iter().collect())
    }

    pub fn get_exported_symbols(&self, file_path: &str) -> Result<Vec<Node>> {
        Ok(self
            .queries
            .get_nodes_by_file(file_path)?
            .into_iter()
            .filter(|node| node.is_exported.unwrap_or(false))
            .collect())
    }

    pub fn find_by_qualified_name(&self, pattern: &str) -> Result<Vec<Node>> {
        let regex = glob_like_regex(pattern);
        let mut matches = Vec::new();
        for kind in [
            NodeKind::Class,
            NodeKind::Function,
            NodeKind::Method,
            NodeKind::Interface,
            NodeKind::TypeAlias,
            NodeKind::Variable,
            NodeKind::Constant,
        ] {
            for node in self.queries.get_nodes_by_kind(kind)? {
                if regex_is_match(&regex, &node.qualified_name) {
                    matches.push(node);
                }
            }
        }
        Ok(matches)
    }

    pub fn get_module_structure(&self) -> Result<BTreeMap<String, Vec<String>>> {
        let mut structure = BTreeMap::new();
        for file in self.queries.get_all_files()? {
            let directory = file
                .path
                .rsplit_once('/')
                .map(|(dir, _)| dir.to_string())
                .unwrap_or_else(|| ".".to_string());
            structure
                .entry(directory)
                .or_insert_with(Vec::new)
                .push(file.path);
        }
        Ok(structure)
    }

    pub fn find_circular_dependencies(&self) -> Result<Vec<Vec<String>>> {
        let files = self.queries.get_all_files()?;
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut recursion_stack = HashSet::new();

        for file in files {
            if !visited.contains(&file.path) {
                self.detect_cycles(
                    &file.path,
                    &mut Vec::new(),
                    &mut visited,
                    &mut recursion_stack,
                    &mut cycles,
                )?;
            }
        }

        Ok(cycles)
    }

    pub fn get_call_graph(&self, node_id: &str, depth: usize) -> Result<Subgraph> {
        let Some(focal) = self.queries.get_node_by_id(node_id)? else {
            return Ok(Subgraph::default());
        };

        let mut nodes = BTreeMap::new();
        let mut edges = Vec::new();
        nodes.insert(focal.id.clone(), focal);

        for item in self.get_callers(node_id, depth)? {
            nodes.entry(item.node.id.clone()).or_insert_with(|| item.node.clone());
            edges.push(item.edge);
        }
        for item in self.get_callees(node_id, depth)? {
            nodes.entry(item.node.id.clone()).or_insert_with(|| item.node.clone());
            edges.push(item.edge);
        }

        Ok(Subgraph {
            nodes: nodes.into_values().collect(),
            edges,
            roots: vec![node_id.to_string()],
        })
    }

    fn collect_related(
        &self,
        node_id: &str,
        max_depth: usize,
        incoming: bool,
        kinds: &[EdgeKind],
    ) -> Result<Vec<NodeEdgeRef>> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::from([(node_id.to_string(), 0usize)]);

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= max_depth || !visited.insert(current_id.clone()) {
                continue;
            }

            let edges = if incoming {
                self.queries.get_incoming_edges(&current_id, Some(kinds))?
            } else {
                self.queries.get_outgoing_edges(&current_id, Some(kinds))?
            };

            for edge in edges {
                let next_id = if incoming { &edge.source } else { &edge.target };
                if let Some(node) = self.queries.get_node_by_id(next_id)? {
                    result.push(NodeEdgeRef {
                        node: node.clone(),
                        edge: edge.clone(),
                    });
                    queue.push_back((node.id, depth + 1));
                }
            }
        }

        Ok(result)
    }

    fn detect_cycles(
        &self,
        file_path: &str,
        path: &mut Vec<String>,
        visited: &mut HashSet<String>,
        recursion_stack: &mut HashSet<String>,
        cycles: &mut Vec<Vec<String>>,
    ) -> Result<()> {
        if recursion_stack.contains(file_path) {
            if let Some(start) = path.iter().position(|item| item == file_path) {
                cycles.push(path[start..].to_vec());
            }
            return Ok(());
        }
        if !visited.insert(file_path.to_string()) {
            return Ok(());
        }

        recursion_stack.insert(file_path.to_string());
        path.push(file_path.to_string());

        for dependency in self.get_file_dependencies(file_path)? {
            self.detect_cycles(&dependency, path, visited, recursion_stack, cycles)?;
        }

        path.pop();
        recursion_stack.remove(file_path);
        Ok(())
    }
}

fn glob_like_regex(pattern: &str) -> String {
    let mut escaped = String::new();
    for ch in pattern.chars() {
        match ch {
            '*' => escaped.push_str(".*"),
            '?' => escaped.push('.'),
            '.' | '+' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    format!("^{escaped}$")
}

fn regex_is_match(pattern: &str, text: &str) -> bool {
    regex::Regex::new(pattern)
        .map(|regex| regex.is_match(text))
        .unwrap_or(false)
}
