use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::directory;
use crate::graph::GraphService;
use crate::query::QueryService;
use crate::types::{Context as NodeContext, EdgeKind, FileRecord, Language, Node, NodeEdgeRef, NodeKind};
use crate::watch::{WatchEvent, WatcherHandle, start_watcher, watch_disabled_reason};

const PROTOCOL_VERSION: &str = "2024-11-05";

pub struct McpServer {
    project_root: Option<PathBuf>,
    watcher: Option<WatcherHandle>,
    client_supports_roots: bool,
    pending_roots_request_id: Option<String>,
    next_request_id: u64,
    outbound_messages: Vec<Value>,
}

impl McpServer {
    pub fn new(project_root: Option<PathBuf>) -> Self {
        Self {
            project_root,
            watcher: None,
            client_supports_roots: false,
            pending_roots_request_id: None,
            next_request_id: 1,
            outbound_messages: Vec::new(),
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = stdout.lock();

        for line in stdin.lock().lines() {
            let line = line.context("failed to read MCP input line")?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let message: Value = match serde_json::from_str(trimmed) {
                Ok(value) => value,
                Err(err) => {
                    write_json(
                        &mut out,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": Value::Null,
                            "error": {
                                "code": -32700,
                                "message": format!("Parse error: {err}")
                            }
                        }),
                    )?;
                    continue;
                }
            };

            if let Some(response) = self.handle_message(message)? {
                write_json(&mut out, &response)?;
            }
            for outbound in self.take_outbound_messages() {
                write_json(&mut out, &outbound)?;
            }
        }

        Ok(())
    }

    fn handle_message(&mut self, message: Value) -> Result<Option<Value>> {
        if self.maybe_handle_response(&message)? {
            return Ok(None);
        }

        let method = message
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Invalid Request: missing method"))?;
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

        let result = match method {
            "initialize" => Some(self.handle_initialize(&params)),
            "notifications/initialized" => None,
            "tools/list" => Some(self.handle_tools_list()),
            "tools/call" => Some(self.handle_tools_call(&params)?),
            "ping" => Some(json!({})),
            _ => {
                let response = if let Some(id) = id {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("Method not found: {method}")
                        }
                    })
                } else {
                    return Ok(None);
                };
                return Ok(Some(response));
            }
        };

        if let (Some(id), Some(result)) = (id, result) {
            return Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            })));
        }

        Ok(None)
    }

    fn handle_initialize(&mut self, params: &Value) -> Value {
        self.client_supports_roots = params
            .get("capabilities")
            .and_then(|value| value.get("roots"))
            .is_some();

        if self.project_root.is_none() {
            if let Some(root_uri) = initialize_root_path(params) {
                self.project_root = resolve_project_root(Some(root_uri.as_path())).ok();
            } else if !self.client_supports_roots {
                self.project_root = resolve_project_root(None).ok();
            }
        }

        self.start_watcher_if_possible();

        if self.project_root.is_none() && self.client_supports_roots {
            let request_id = format!("cg-srv-{}", self.next_request_id);
            self.next_request_id += 1;
            self.pending_roots_request_id = Some(request_id.clone());
            self.outbound_messages.push(json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "roots/list"
            }));
        }

        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": "codegraph",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": "Use codegraph_* tools for structural code questions before falling back to file reads."
        })
    }

    fn handle_tools_list(&self) -> Value {
        json!({
            "tools": tool_definitions()
        })
    }

    fn handle_tools_call(&self, params: &Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing tool name"))?;
        let arguments = params
            .get("arguments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let result = match name {
            "codegraph_search" => self.tool_search(&arguments)?,
            "codegraph_context" => self.tool_context(&arguments)?,
            "codegraph_callers" => self.tool_callers(&arguments)?,
            "codegraph_callees" => self.tool_callees(&arguments)?,
            "codegraph_impact" => self.tool_impact(&arguments)?,
            "codegraph_explore" => self.tool_explore(&arguments)?,
            "codegraph_node" => self.tool_node(&arguments)?,
            "codegraph_status" => self.tool_status(&arguments)?,
            "codegraph_files" => self.tool_files(&arguments)?,
            _ => tool_error(format!("Unknown tool: {name}")),
        };

        Ok(result)
    }

    fn tool_search(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let query = required_string(args, "query")?;
        let limit = optional_u64(args, "limit").unwrap_or(10).clamp(1, 100) as usize;
        let kind = optional_string(args, "kind");
        let language = optional_string(args, "language");
        let service = self.open_query_service(optional_string(args, "projectPath"))?;
        let results = service.search_nodes(&query, kind.as_deref(), language.as_deref(), limit)?;

        if results.is_empty() {
            return Ok(tool_text(format!("No results found for \"{query}\"")));
        }

        Ok(tool_text(format_node_lines(&results)))
    }

    fn tool_context(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let task = required_string(args, "task")?;
        let limit = optional_u64(args, "maxNodes").unwrap_or(5).clamp(1, 20) as usize;
        let service = self.open_query_service(optional_string(args, "projectPath"))?;
        let graph = GraphService::new(&service);
        let matches = service.search_nodes(&task, None, None, limit)?;

        if matches.is_empty() {
            return Ok(tool_text(format!("No relevant graph context found for \"{task}\"")));
        }

        let mut sections = Vec::new();
        sections.push(format!("Task: {task}"));
        sections.push(String::new());
        sections.push("Top matching symbols:".to_string());

        for node in matches {
            let context = graph.get_context(&node.id)?;
            sections.push(format_context_block(&context));
        }

        Ok(tool_text(sections.join("\n")))
    }

    fn tool_callers(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        self.tool_related(args, "symbol", true)
    }

    fn tool_callees(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        self.tool_related(args, "symbol", false)
    }

    fn tool_related(
        &self,
        args: &serde_json::Map<String, Value>,
        field: &str,
        callers: bool,
    ) -> Result<Value> {
        let symbol = required_string(args, field)?;
        let limit = optional_u64(args, "limit").unwrap_or(20).clamp(1, 100) as usize;
        let service = self.open_query_service(optional_string(args, "projectPath"))?;
        let graph = GraphService::new(&service);
        let Some(node) = select_best_match(&service, &symbol)? else {
            return Ok(tool_text(format!("Symbol \"{symbol}\" not found in the codebase")));
        };

        let related = if callers {
            graph.get_callers(&node.id, 2)?
        } else {
            graph.get_callees(&node.id, 2)?
        };
        if related.is_empty() {
            let label = if callers { "callers" } else { "callees" };
            return Ok(tool_text(format!("No {label} found for \"{symbol}\"")));
        }

        let mut lines = Vec::new();
        lines.push(format!(
            "{} of {}",
            if callers { "Callers" } else { "Callees" },
            node.qualified_name
        ));
        for item in related.into_iter().take(limit) {
            lines.push(format_related_line(&item));
        }

        Ok(tool_text(lines.join("\n")))
    }

    fn tool_impact(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let symbol = required_string(args, "symbol")?;
        let service = self.open_query_service(optional_string(args, "projectPath"))?;
        let graph = GraphService::new(&service);
        let Some(node) = select_best_match(&service, &symbol)? else {
            return Ok(tool_text(format!("Symbol \"{symbol}\" not found in the codebase")));
        };

        let callers = graph.get_callers(&node.id, 2)?;
        let callees = graph.get_callees(&node.id, 2)?;
        let context = graph.get_context(&node.id)?;

        let payload = json!({
            "focal": context.focal,
            "file": context.focal.file_path,
            "callers": callers,
            "callees": callees,
            "children": context.children,
            "imports": context.imports,
        });
        Ok(tool_text(serde_json::to_string_pretty(&payload)?))
    }

    fn tool_explore(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let project_root = self.resolve_target_root(optional_string(args, "projectPath"))?;
        let service = QueryService::open(&project_root)?;
        let graph = GraphService::new(&service);

        let query = required_string(args, "query")?;
        let file_count = service.get_stats().map(|s| s.file_count).unwrap_or(0);
        let budget = get_explore_budget(file_count);
        let max_files = optional_u64(args, "maxFiles")
            .map(|v| v as usize)
            .unwrap_or(budget.default_max_files)
            .clamp(1, 20);

        const MAX_NODES: usize = 200;

        // Step 1: find entry-point nodes via search
        let entry_nodes = service.search_nodes(&query, None, None, 8)?;
        if entry_nodes.is_empty() {
            return Ok(tool_text(format!("No relevant code found for \"{query}\"")));
        }
        let entry_ids: HashSet<String> = entry_nodes.iter().map(|n| n.id.clone()).collect();

        // Step 2: BFS expansion – callers + callees + children from each entry
        let mut all_nodes: HashMap<String, Node> =
            entry_nodes.iter().map(|n| (n.id.clone(), n.clone())).collect();
        let mut raw_edges: Vec<(String, String, String)> = Vec::new(); // (src_id, tgt_id, kind)
        let mut connected_to_entry: HashSet<String> = HashSet::new();

        'expand: for entry in &entry_nodes {
            let callees = graph.get_callees(&entry.id, 2)?;
            for item in callees {
                if all_nodes.len() >= MAX_NODES { break 'expand; }
                connected_to_entry.insert(item.node.id.clone());
                if !matches!(item.edge.kind, EdgeKind::Contains) {
                    raw_edges.push((
                        item.edge.source.clone(),
                        item.edge.target.clone(),
                        enum_label(&item.edge.kind).unwrap_or_default(),
                    ));
                }
                all_nodes.entry(item.node.id.clone()).or_insert(item.node);
            }
            let callers = graph.get_callers(&entry.id, 2)?;
            for item in callers {
                if all_nodes.len() >= MAX_NODES { break 'expand; }
                connected_to_entry.insert(item.node.id.clone());
                if !matches!(item.edge.kind, EdgeKind::Contains) {
                    raw_edges.push((
                        item.edge.source.clone(),
                        item.edge.target.clone(),
                        enum_label(&item.edge.kind).unwrap_or_default(),
                    ));
                }
                all_nodes.entry(item.node.id.clone()).or_insert(item.node);
            }
            for child in graph.get_children(&entry.id)? {
                if all_nodes.len() >= MAX_NODES { break 'expand; }
                connected_to_entry.insert(child.id.clone());
                all_nodes.entry(child.id.clone()).or_insert(child);
            }
        }

        // Step 3: group nodes by file with relevance scoring
        let mut file_groups: HashMap<String, (Vec<Node>, u32)> = HashMap::new();
        for node in all_nodes.values() {
            if matches!(node.kind, NodeKind::Import | NodeKind::Export) { continue; }
            let score = if entry_ids.contains(&node.id) { 10u32 }
                else if connected_to_entry.contains(&node.id) { 3 }
                else { 1 };
            let entry = file_groups.entry(node.file_path.clone()).or_insert_with(|| (Vec::new(), 0));
            entry.0.push(node.clone());
            entry.1 += score;
        }

        let total_file_count = file_groups.len();

        // Query-term relevance helpers
        let query_terms: Vec<String> = query.to_lowercase().split_whitespace()
            .filter(|t| t.len() >= 3)
            .map(|t| t.to_string())
            .collect();
        let has_query_relevance = |file_path: &str, nodes: &[Node]| -> bool {
            let fp = file_path.to_lowercase();
            query_terms.iter().any(|t| fp.contains(t.as_str()))
                || nodes.iter().any(|n| {
                    query_terms.iter().any(|t| n.name.to_lowercase().contains(t.as_str()))
                })
        };
        let is_low_value = |p: &str| -> bool {
            let pl = p.to_lowercase();
            pl.contains("/test") || pl.contains("__test") || pl.contains("/spec")
                || pl.contains("/icons") || pl.contains("/i18n")
        };

        let (mut relevant, peripheral): (Vec<(String, (Vec<Node>, u32))>, Vec<_>) =
            file_groups.into_iter().partition(|(_, (_, s))| *s >= 3);

        relevant.sort_by(|a, b| {
            let a_rel = has_query_relevance(&a.0, &a.1.0);
            let b_rel = has_query_relevance(&b.0, &b.1.0);
            if a_rel != b_rel {
                return if a_rel { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
            }
            let a_low = is_low_value(&a.0);
            let b_low = is_low_value(&b.0);
            if a_low != b_low {
                return if a_low { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less };
            }
            b.1.1.cmp(&a.1.1).then(b.1.0.len().cmp(&a.1.0.len()))
        });

        // Build relationship edges grouped by kind (post-BFS so all names are known)
        let mut by_kind: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        {
            let mut seen: HashSet<String> = HashSet::new();
            for (src_id, tgt_id, kind) in &raw_edges {
                let src = all_nodes.get(src_id).map(|n| n.name.as_str()).unwrap_or("");
                let tgt = all_nodes.get(tgt_id).map(|n| n.name.as_str()).unwrap_or("");
                if src.is_empty() || tgt.is_empty() { continue; }
                let key = format!("{src}->{tgt}@{kind}");
                if !seen.insert(key) { continue; }
                by_kind.entry(kind.clone()).or_default().push((src.to_string(), tgt.to_string()));
            }
        }

        // Step 4: assemble output
        let mut lines: Vec<String> = vec![
            format!("## Exploration: {query}"),
            String::new(),
            format!("Found {} symbols across {total_file_count} files.", all_nodes.len()),
            String::new(),
        ];

        if budget.include_relationships && !by_kind.is_empty() {
            lines.push("### Relationships".to_string());
            lines.push(String::new());
            for (kind, edges) in &by_kind {
                let cap = budget.max_edges_per_relationship_kind;
                lines.push(format!("**{kind}:**"));
                for (src, tgt) in edges.iter().take(cap) {
                    lines.push(format!("- {src} → {tgt}"));
                }
                if edges.len() > cap {
                    lines.push(format!("- ... and {} more", edges.len() - cap));
                }
                lines.push(String::new());
            }
        }

        lines.push("### Source Code".to_string());
        lines.push(String::new());

        let mut total_chars: usize = lines.iter().map(|l| l.len() + 1).sum();
        let mut files_included = 0usize;
        let mut any_file_trimmed = false;

        const CONTEXT_PADDING: i64 = 3;
        const GAP_MARKER: &str = "\n\n... (gap) ...\n\n";

        let is_envelope = |kind: &NodeKind| {
            matches!(
                kind,
                NodeKind::File | NodeKind::Module | NodeKind::Class | NodeKind::Struct
                    | NodeKind::Interface | NodeKind::Enum | NodeKind::Namespace
                    | NodeKind::Protocol | NodeKind::Trait | NodeKind::Component
            )
        };

        'files: for (file_path, (nodes, _)) in &relevant {
            if files_included >= max_files { break; }
            if total_chars > budget.max_output_chars * 9 / 10 { break; }

            let abs_path = project_root.join(file_path);
            let file_content = match std::fs::read_to_string(&abs_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let file_lines: Vec<&str> = file_content.split('\n').collect();
            let total_file_lines = file_lines.len() as i64;

            let lang = nodes.first().map(|n| language_fence_name(&n.language)).unwrap_or("");

            // Build symbol ranges, dropping large envelope nodes
            let mut ranges: Vec<(i64, i64, String, u32)> = Vec::new();
            for node in nodes {
                if node.start_line <= 0 || node.end_line <= 0 { continue; }
                if is_envelope(&node.kind) && (node.end_line - node.start_line + 1) * 2 > total_file_lines {
                    continue;
                }
                let importance = if entry_ids.contains(&node.id) { 10u32 }
                    else if connected_to_entry.contains(&node.id) { 3 }
                    else { 1 };
                ranges.push((
                    node.start_line, node.end_line,
                    format!("{}({})", node.name, enum_label(&node.kind).unwrap_or_default()),
                    importance,
                ));
            }
            if ranges.is_empty() { continue; }
            ranges.sort_by_key(|r| r.0);

            // Merge into clusters within gap threshold
            let gap_threshold = budget.gap_threshold;
            let mut clusters: Vec<(i64, i64, Vec<String>, u32, u32)> = Vec::new();
            let first = &ranges[0];
            let mut cur = (first.0, first.1, vec![first.2.clone()], first.3, first.3);
            for r in &ranges[1..] {
                if r.0 <= cur.1 + gap_threshold {
                    cur.1 = cur.1.max(r.1);
                    cur.2.push(r.2.clone());
                    cur.3 += r.3;
                    cur.4 = cur.4.max(r.3);
                } else {
                    clusters.push(cur);
                    cur = (r.0, r.1, vec![r.2.clone()], r.3, r.3);
                }
            }
            clusters.push(cur);

            // Source section builder (line-numbered, cat -n style)
            let build_section = |start: i64, end: i64| -> String {
                let s = ((start - 1 - CONTEXT_PADDING).max(0)) as usize;
                let e = ((end + CONTEXT_PADDING).min(total_file_lines)) as usize;
                let e = e.min(file_lines.len());
                file_lines[s..e].iter().enumerate()
                    .map(|(i, l)| format!("{}\t{l}", s + 1 + i))
                    .collect::<Vec<_>>()
                    .join("\n")
            };

            // Rank clusters: highest max_importance, then density, then smaller span
            let mut ranked: Vec<usize> = (0..clusters.len()).collect();
            ranked.sort_by(|&a, &b| {
                let ca = &clusters[a]; let cb = &clusters[b];
                if cb.4 != ca.4 { return cb.4.cmp(&ca.4); }
                let a_span = (ca.1 - ca.0 + 1).max(1) as f64;
                let b_span = (cb.1 - cb.0 + 1).max(1) as f64;
                let a_d = ca.3 as f64 / a_span;
                let b_d = cb.3 as f64 / b_span;
                b_d.partial_cmp(&a_d).unwrap_or(std::cmp::Ordering::Equal)
                    .then(cb.3.cmp(&ca.3))
                    .then(a_span.partial_cmp(&b_span).unwrap_or(std::cmp::Ordering::Equal))
            });

            // Pick clusters within per-file char budget
            let mut chosen: HashSet<usize> = HashSet::new();
            let mut projected = 0usize;
            for &idx in &ranked {
                let c = &clusters[idx];
                let section = build_section(c.0, c.1);
                let cost = section.len() + if chosen.is_empty() { 0 } else { GAP_MARKER.len() };
                if chosen.is_empty() {
                    chosen.insert(idx);
                    projected += cost;
                } else if projected + cost <= budget.max_chars_per_file {
                    chosen.insert(idx);
                    projected += cost;
                }
            }

            // Emit chosen clusters in source order
            let mut file_section = String::new();
            let mut all_symbols: Vec<String> = Vec::new();
            let mut file_trimmed = chosen.len() < clusters.len();
            for (i, cluster) in clusters.iter().enumerate() {
                if !chosen.contains(&i) { continue; }
                if !file_section.is_empty() { file_section.push_str(GAP_MARKER); }
                file_section.push_str(&build_section(cluster.0, cluster.1));
                all_symbols.extend_from_slice(&cluster.2);
            }
            if file_section.len() > budget.max_chars_per_file {
                file_section.truncate(budget.max_chars_per_file);
                file_section.push_str("\n... (trimmed) ...");
                file_trimmed = true;
            }
            if file_trimmed { any_file_trimmed = true; }

            // File header: top symbols by frequency
            let mut sym_counts: HashMap<String, usize> = HashMap::new();
            for s in &all_symbols { *sym_counts.entry(s.clone()).or_insert(0) += 1; }
            let mut sorted_syms: Vec<(String, usize)> = sym_counts.into_iter().collect();
            sorted_syms.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let cap = budget.max_symbols_in_file_header;
            let shown: Vec<&str> = sorted_syms.iter().take(cap).map(|(s, _)| s.as_str()).collect();
            let omitted = sorted_syms.len().saturating_sub(cap);
            let header_suffix = if omitted > 0 {
                format!("{}, +{omitted} more", shown.join(", "))
            } else {
                shown.join(", ")
            };
            let file_header = format!("#### {file_path} — {header_suffix}");

            // Total output cap check
            let cost = file_section.len() + 200;
            if total_chars + cost > budget.max_output_chars {
                let remaining = budget.max_output_chars.saturating_sub(total_chars + 200);
                if remaining < 500 { break 'files; }
                let trim_at = remaining.min(file_section.len());
                let trimmed = format!("{}\n... (trimmed) ...", &file_section[..trim_at]);
                lines.extend([file_header, String::new(), format!("```{lang}"), trimmed, "```".to_string(), String::new()]);
                files_included += 1;
                any_file_trimmed = true;
                break 'files;
            }

            lines.extend([file_header, String::new(), format!("```{lang}"), file_section, "```".to_string(), String::new()]);
            total_chars += cost;
            files_included += 1;
        }

        // Additional relevant files not shown
        if budget.include_additional_files {
            let additional: Vec<(String, Vec<String>)> = relevant
                .iter()
                .skip(files_included)
                .chain(peripheral.iter())
                .take(10)
                .map(|(path, (nodes, _))| {
                    let syms: Vec<String> = nodes.iter().take(5)
                        .map(|n| format!("{}:{}", n.name, n.start_line))
                        .collect();
                    (path.clone(), syms)
                })
                .collect();

            if !additional.is_empty() {
                lines.push("### Additional relevant files (not shown)".to_string());
                lines.push(String::new());
                for (path, syms) in &additional {
                    lines.push(format!("- {path}: {}", syms.join(", ")));
                }
                let total_extra = relevant.len().saturating_sub(files_included) + peripheral.len();
                if total_extra > 10 {
                    lines.push(format!("- ... and {} more files", total_extra - 10));
                }
            }
        }

        // Completeness / trim signal
        if budget.include_completeness_signal {
            lines.push(String::new());
            lines.push("---".to_string());
            lines.push(format!(
                "> **Complete source code is included above for {files_included} files.** \
                 You do NOT need to re-read these files — the relevant sections are already shown. \
                 Only use Read/Grep for files listed under \"Additional relevant files\" if you need more detail."
            ));
        } else if any_file_trimmed {
            lines.push(String::new());
            lines.push("> Some file sections were trimmed. Use `codegraph_node` or Read for full source.".to_string());
        }

        // Call budget note
        if budget.include_budget_note {
            let call_budget = get_explore_call_budget(file_count);
            lines.push(String::new());
            lines.push(format!(
                "> **Explore budget: {call_budget} calls max for this project ({file_count} files indexed).** \
                 Stop exploring and synthesize your answer once you've used {call_budget} calls."
            ));
        }

        // Hard final cap
        let output = lines.join("\n");
        if output.len() > budget.max_output_chars {
            let cut = budget.max_output_chars;
            let safe = output[..cut].rfind('\n')
                .filter(|&i| i > cut * 4 / 5)
                .map_or(&output[..cut], |i| &output[..i]);
            return Ok(tool_text(format!(
                "{safe}\n\n... (explore output truncated — use codegraph_node or Read for more)"
            )));
        }

        Ok(tool_text(output))
    }

    fn tool_node(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let service = self.open_query_service(optional_string(args, "projectPath"))?;
        let node = if let Some(node_id) = optional_string(args, "nodeId") {
            service.get_node_by_id(&node_id)?
        } else if let Some(symbol) = optional_string(args, "symbol") {
            select_best_match(&service, &symbol)?
        } else {
            bail!("Missing symbol or nodeId");
        };

        let Some(node) = node else {
            return Ok(tool_text("Node not found".to_string()));
        };

        Ok(tool_text(serde_json::to_string_pretty(&node)?))
    }

    fn tool_status(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let project_root = self.resolve_target_root(optional_string(args, "projectPath"))?;
        let initialized = directory::is_initialized(&project_root);
        if !initialized {
            return Ok(tool_text(format!(
                "CodeGraph is not initialized in {}",
                project_root.display()
            )));
        }

        let service = QueryService::open(&project_root)?;
        let stats = service.get_stats()?;
        let payload = json!({
            "projectPath": project_root,
            "initialized": true,
            "database": service.database_info().path,
            "stats": stats,
        });
        Ok(tool_text(serde_json::to_string_pretty(&payload)?))
    }

    fn tool_files(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let service = self.open_query_service(optional_string(args, "projectPath"))?;
        let prefix = optional_string(args, "pathPrefix");
        let limit = optional_u64(args, "limit").unwrap_or(200).clamp(1, 1000) as usize;
        let mut files = service.get_all_files()?;
        if let Some(prefix) = prefix {
            files.retain(|file| file.path.starts_with(&prefix));
        }
        if files.len() > limit {
            files.truncate(limit);
        }

        Ok(tool_text(format_file_lines(&files)))
    }

    fn open_query_service(&self, project_path: Option<String>) -> Result<QueryService> {
        let root = self.resolve_target_root(project_path)?;
        QueryService::open(&root)
    }

    fn resolve_target_root(&self, project_path: Option<String>) -> Result<PathBuf> {
        if let Some(project_path) = project_path {
            return resolve_project_root(Some(Path::new(&project_path)));
        }
        if let Some(project_root) = &self.project_root {
            return Ok(project_root.clone());
        }
        resolve_project_root(None)
    }

    fn maybe_handle_response(&mut self, message: &Value) -> Result<bool> {
        let Some(id) = message.get("id").and_then(Value::as_str) else {
            return Ok(false);
        };
        let Some(pending_id) = self.pending_roots_request_id.as_deref() else {
            return Ok(false);
        };
        if id != pending_id {
            return Ok(false);
        }

        self.pending_roots_request_id = None;
        if let Some(result) = message.get("result") {
            if let Some(root_path) = first_root_path(result) {
                self.project_root = resolve_project_root(Some(root_path.as_path())).ok();
                self.start_watcher_if_possible();
            }
        }
        Ok(true)
    }

    fn take_outbound_messages(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.outbound_messages)
    }

    /// Like `start()` but accepts any BufRead reader + Write writer instead of
    /// stdin/stdout. Used by the UI's TCP listener so each connection gets its
    /// own server instance.
    pub fn start_with_io(
        &mut self,
        reader: impl io::BufRead,
        mut writer: impl io::Write,
    ) -> Result<()> {
        for line in reader.lines() {
            let line = line.context("failed to read MCP input line")?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let message: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(err) => {
                    write_json(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": Value::Null,
                            "error": { "code": -32700, "message": format!("Parse error: {err}") }
                        }),
                    )?;
                    continue;
                }
            };
            if let Some(response) = self.handle_message(message)? {
                write_json(&mut writer, &response)?;
            }
            for outbound in self.take_outbound_messages() {
                write_json(&mut writer, &outbound)?;
            }
        }
        Ok(())
    }

    fn start_watcher_if_possible(&mut self) {
        if self.watcher.is_some() {
            return;
        }
        let Some(project_root) = self.project_root.clone() else {
            return;
        };
        if !directory::is_initialized(&project_root) {
            return;
        }
        if let Some(reason) = watch_disabled_reason(&project_root) {
            eprintln!(
                "[CodeGraph MCP] File watcher disabled — {}. The graph will not auto-update; run `codegraph sync` to refresh.",
                reason
            );
            return;
        }

        match start_watcher(
            project_root,
            2_000,
            |event: WatchEvent| {
                if event.files_changed > 0 {
                    eprintln!(
                        "[CodeGraph MCP] Auto-synced {} file(s) in {}ms",
                        event.files_changed, event.duration_ms
                    );
                }
            },
            |err| {
                eprintln!("[CodeGraph MCP] Auto-sync error: {err:#}");
            },
        ) {
            Ok(handle) => {
                eprintln!("[CodeGraph MCP] File watcher active — graph will auto-sync on changes");
                self.watcher = Some(handle);
            }
            Err(err) => {
                eprintln!(
                    "[CodeGraph MCP] File watcher unavailable on this platform — run `codegraph sync` to refresh the graph after changes ({err:#})"
                );
            }
        }
    }
}

fn resolve_project_root(path: Option<&Path>) -> Result<PathBuf> {
    let start = path
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
    let absolute = std::fs::canonicalize(&start).unwrap_or(start);

    if directory::is_initialized(&absolute) {
        return Ok(absolute);
    }

    let mut current = absolute.as_path();
    loop {
        if directory::is_initialized(current) {
            return Ok(current.to_path_buf());
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }

    Ok(absolute)
}

fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let trimmed = uri.strip_prefix("file://")?;
    let candidate = if cfg!(windows) && trimmed.starts_with('/') && trimmed.get(2..3) == Some(":")
    {
        &trimmed[1..]
    } else {
        trimmed
    };
    Some(PathBuf::from(candidate))
}

fn initialize_root_path(params: &Value) -> Option<PathBuf> {
    params
        .get("rootUri")
        .and_then(Value::as_str)
        .and_then(file_uri_to_path)
        .or_else(|| {
            params
                .get("workspaceFolders")
                .and_then(Value::as_array)
                .and_then(|folders| folders.first())
                .and_then(|item| item.get("uri"))
                .and_then(Value::as_str)
                .and_then(file_uri_to_path)
        })
}

fn first_root_path(result: &Value) -> Option<PathBuf> {
    result
        .get("roots")
        .and_then(Value::as_array)
        .and_then(|roots| roots.first())
        .and_then(|item| item.get("uri"))
        .and_then(Value::as_str)
        .and_then(file_uri_to_path)
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool_schema("codegraph_search", "Quick symbol search by name. Returns locations only.", json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Symbol name or partial name"},
                "kind": {"type": "string", "description": "Optional node kind filter"},
                "language": {"type": "string", "description": "Optional language filter"},
                "limit": {"type": "number", "description": "Maximum results"},
                "projectPath": {"type": "string", "description": "Optional alternate initialized project root"}
            },
            "required": ["query"]
        })),
        tool_schema("codegraph_context", "Build lightweight graph context for a task.", json!({
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "Task or area to inspect"},
                "maxNodes": {"type": "number", "description": "Maximum matched symbols to expand"},
                "projectPath": {"type": "string", "description": "Optional alternate initialized project root"}
            },
            "required": ["task"]
        })),
        tool_schema("codegraph_callers", "Find callers of a symbol.", json!({
            "type": "object",
            "properties": {
                "symbol": {"type": "string", "description": "Qualified or simple symbol name"},
                "limit": {"type": "number", "description": "Maximum results"},
                "projectPath": {"type": "string", "description": "Optional alternate initialized project root"}
            },
            "required": ["symbol"]
        })),
        tool_schema("codegraph_callees", "Find what a symbol calls.", json!({
            "type": "object",
            "properties": {
                "symbol": {"type": "string", "description": "Qualified or simple symbol name"},
                "limit": {"type": "number", "description": "Maximum results"},
                "projectPath": {"type": "string", "description": "Optional alternate initialized project root"}
            },
            "required": ["symbol"]
        })),
        tool_schema("codegraph_impact", "Summarize immediate graph impact around a symbol.", json!({
            "type": "object",
            "properties": {
                "symbol": {"type": "string", "description": "Qualified or simple symbol name"},
                "projectPath": {"type": "string", "description": "Optional alternate initialized project root"}
            },
            "required": ["symbol"]
        })),
        tool_schema("codegraph_explore", "PRIMARY TOOL — deep exploration returning source code for related symbols grouped by file, plus a relationships map. Call this first for any \"how does X work\" question.", json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Symbol names, file names, or short code terms to explore"},
                "maxFiles": {"type": "number", "description": "Max files to include source for (adaptive default based on project size)"},
                "projectPath": {"type": "string", "description": "Optional alternate initialized project root"}
            },
            "required": ["query"]
        })),
        tool_schema("codegraph_node", "Get a single node by symbol or node id.", json!({
            "type": "object",
            "properties": {
                "symbol": {"type": "string", "description": "Qualified or simple symbol name"},
                "nodeId": {"type": "string", "description": "Node id"},
                "projectPath": {"type": "string", "description": "Optional alternate initialized project root"}
            }
        })),
        tool_schema("codegraph_status", "Report index status and graph counts.", json!({
            "type": "object",
            "properties": {
                "projectPath": {"type": "string", "description": "Optional alternate initialized project root"}
            }
        })),
        tool_schema("codegraph_files", "List indexed files.", json!({
            "type": "object",
            "properties": {
                "pathPrefix": {"type": "string", "description": "Optional path prefix filter"},
                "limit": {"type": "number", "description": "Maximum files to list"},
                "projectPath": {"type": "string", "description": "Optional alternate initialized project root"}
            }
        })),
    ]
}

fn tool_schema(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

fn tool_text(text: String) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ]
    })
}

fn tool_error(message: String) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": message
            }
        ],
        "isError": true
    })
}

fn required_string(args: &serde_json::Map<String, Value>, key: &str) -> Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("Missing required string field: {key}"))
}

fn optional_string(args: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn optional_u64(args: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn select_best_match(service: &QueryService, symbol: &str) -> Result<Option<Node>> {
    let mut matches = service.search_nodes(symbol, None, None, 20)?;
    if matches.is_empty() {
        return Ok(None);
    }

    matches.sort_by_key(|node| {
        (
            node.name != symbol,
            node.qualified_name != symbol,
            node.qualified_name.ends_with(symbol).not(),
            node.start_line,
        )
    });
    Ok(matches.into_iter().next())
}

fn format_node_lines(nodes: &[Node]) -> String {
    nodes.iter()
        .map(|node| {
            format!(
                "{} [{}] {}:{}-{}",
                node.qualified_name,
                serde_json::to_string(&node.kind).unwrap_or_else(|_| "\"unknown\"".to_string()).trim_matches('"').to_string(),
                node.file_path,
                node.start_line,
                node.end_line
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_context_block(context: &NodeContext) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "- {} [{}] {}:{}-{}",
        context.focal.qualified_name,
        enum_label(&context.focal.kind).unwrap_or_else(|_| "unknown".to_string()),
        context.focal.file_path,
        context.focal.start_line,
        context.focal.end_line
    ));
    push_summary(&mut lines, "ancestors", &context.ancestors);
    push_summary_ref(&mut lines, "incoming", &context.incoming_refs);
    push_summary_ref(&mut lines, "outgoing", &context.outgoing_refs);
    push_summary(&mut lines, "children", &context.children);
    lines.join("\n")
}

fn push_summary(lines: &mut Vec<String>, label: &str, nodes: &[Node]) {
    if nodes.is_empty() {
        return;
    }
    let summary = nodes
        .iter()
        .take(5)
        .map(|node| node.qualified_name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!("  {label}: {summary}"));
}

fn push_summary_ref(lines: &mut Vec<String>, label: &str, refs: &[NodeEdgeRef]) {
    if refs.is_empty() {
        return;
    }
    let summary = refs
        .iter()
        .take(5)
        .map(format_related_line)
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!("  {label}: {summary}"));
}

fn format_related_line(item: &NodeEdgeRef) -> String {
    format!(
        "{} [{} via {}]",
        item.node.qualified_name,
        enum_label(&item.node.kind).unwrap_or_else(|_| "unknown".to_string()),
        enum_label(&item.edge.kind).unwrap_or_else(|_| "unknown".to_string())
    )
}

fn format_file_lines(files: &[FileRecord]) -> String {
    files.iter()
        .map(|file| {
            format!(
                "{} [{}] nodes={}",
                file.path,
                enum_label(&file.language).unwrap_or_else(|_| "unknown".to_string()),
                file.node_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn enum_label<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?.trim_matches('"').to_string())
}

struct ExploreBudget {
    max_output_chars: usize,
    default_max_files: usize,
    max_chars_per_file: usize,
    gap_threshold: i64,
    max_symbols_in_file_header: usize,
    max_edges_per_relationship_kind: usize,
    include_relationships: bool,
    include_additional_files: bool,
    include_completeness_signal: bool,
    include_budget_note: bool,
}

fn get_explore_budget(file_count: i64) -> ExploreBudget {
    if file_count < 500 {
        ExploreBudget {
            max_output_chars: 18_000,
            default_max_files: 5,
            max_chars_per_file: 3_800,
            gap_threshold: 8,
            max_symbols_in_file_header: 6,
            max_edges_per_relationship_kind: 6,
            include_relationships: true,
            include_additional_files: false,
            include_completeness_signal: false,
            include_budget_note: false,
        }
    } else if file_count < 5_000 {
        ExploreBudget {
            max_output_chars: 13_000,
            default_max_files: 6,
            max_chars_per_file: 2_500,
            gap_threshold: 10,
            max_symbols_in_file_header: 8,
            max_edges_per_relationship_kind: 8,
            include_relationships: true,
            include_additional_files: true,
            include_completeness_signal: true,
            include_budget_note: true,
        }
    } else if file_count < 15_000 {
        ExploreBudget {
            max_output_chars: 35_000,
            default_max_files: 12,
            max_chars_per_file: 7_000,
            gap_threshold: 15,
            max_symbols_in_file_header: 15,
            max_edges_per_relationship_kind: 15,
            include_relationships: true,
            include_additional_files: true,
            include_completeness_signal: true,
            include_budget_note: true,
        }
    } else {
        ExploreBudget {
            max_output_chars: 38_000,
            default_max_files: 14,
            max_chars_per_file: 7_000,
            gap_threshold: 15,
            max_symbols_in_file_header: 15,
            max_edges_per_relationship_kind: 15,
            include_relationships: true,
            include_additional_files: true,
            include_completeness_signal: true,
            include_budget_note: true,
        }
    }
}

fn get_explore_call_budget(file_count: i64) -> u32 {
    if file_count < 500 { 3 }
    else if file_count < 5_000 { 5 }
    else if file_count < 15_000 { 8 }
    else { 10 }
}

fn language_fence_name(lang: &Language) -> &'static str {
    match lang {
        Language::Typescript | Language::Tsx => "typescript",
        Language::Javascript | Language::Jsx => "javascript",
        Language::Python => "python",
        Language::Go => "go",
        Language::Rust => "rust",
        Language::Java => "java",
        Language::C => "c",
        Language::Cpp => "cpp",
        Language::Csharp => "csharp",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Swift => "swift",
        Language::Kotlin => "kotlin",
        Language::Dart => "dart",
        Language::Scala => "scala",
        Language::Lua | Language::Luau => "lua",
        Language::Svelte => "svelte",
        Language::Vue => "vue",
        Language::Liquid => "liquid",
        Language::Pascal => "",
        Language::Unknown => "",
    }
}

fn write_json(out: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

impl Drop for McpServer {
    fn drop(&mut self) {
        if let Some(watcher) = &mut self.watcher {
            watcher.stop();
        }
    }
}

trait BoolNot {
    fn not(self) -> bool;
}

impl BoolNot for bool {
    fn not(self) -> bool {
        !self
    }
}
