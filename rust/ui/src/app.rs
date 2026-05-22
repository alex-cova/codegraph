use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use codegraph_rs::graph::GraphService;
use codegraph_rs::query::QueryService;
use codegraph_rs::types::{EdgeKind, Node, NodeEdgeRef, NodeKind};
use iced::widget::{button, column, container, row, text};
use iced::{Element, Length, Subscription, Task, Theme};

use codegraph_rs::config::{create_default_config, load_config};
use codegraph_rs::extraction::sync_project;

use crate::db::load_graph;
use crate::graph::canvas;
use crate::graph::layout;
use crate::graph::types::{
    GraphEdge, GraphNode, Viewport, CANVAS_W, MAX_VISIBLE_NODES,
};
use crate::mcp_server::{self, McpHandle};
use crate::panels::{detail, sidebar};
use crate::theme;

// ── Messages ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Message {
    SearchChanged(String),
    FilterChanged(FilterKind),
    UnusedFilterChanged(UnusedFilterKind),
    NodeSelected(String),
    NodeDeselected,
    NodeMoved { id: String, x: f32, y: f32 },
    Pan { dx: f32, dy: f32 },
    Zoom { factor: f32, center_x: f32, center_y: f32 },
    LayoutTick,
    ToggleMcp,
    Sync,
    SyncComplete(Result<String, String>),
}

#[derive(Debug, Clone)]
pub enum FilterKind {
    ToggleKind(NodeKind),
}

#[derive(Debug, Clone)]
pub enum UnusedFilterKind {
    NoInbound,
    Unexported,
    OrphanFiles,
    DeadRoutes,
}

// ── Filters ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Filters {
    pub hidden_kinds: HashSet<NodeKind>,
    pub unused_no_inbound: bool,
    pub unused_unexported: bool,
    pub unused_orphan_files: bool,
    pub unused_dead_routes: bool,
}

impl Default for Filters {
    fn default() -> Self {
        let mut hidden_kinds = HashSet::new();
        // Hide low-signal node kinds by default
        hidden_kinds.insert(NodeKind::Parameter);
        hidden_kinds.insert(NodeKind::Import);
        hidden_kinds.insert(NodeKind::Export);
        hidden_kinds.insert(NodeKind::EnumMember);
        hidden_kinds.insert(NodeKind::Property);
        hidden_kinds.insert(NodeKind::Field);
        Self {
            hidden_kinds,
            unused_no_inbound: false,
            unused_unexported: false,
            unused_orphan_files: false,
            unused_dead_routes: false,
        }
    }
}

impl Filters {
    pub fn any_unused(&self) -> bool {
        self.unused_no_inbound
            || self.unused_unexported
            || self.unused_orphan_files
            || self.unused_dead_routes
    }
}

// ── Application ──────────────────────────────────────────────────────────────

pub struct App {
    pub project_path: PathBuf,

    // All data from DB
    all_nodes: Vec<Node>,
    all_edges: Vec<codegraph_rs::types::Edge>,
    unused_no_inbound: HashSet<String>,
    unused_unexported: HashSet<String>,
    orphan_files: HashSet<String>,
    dead_routes: HashSet<String>,

    // Visible subset for the graph
    pub visible_nodes: Vec<GraphNode>,
    pub visible_edges: Vec<GraphEdge>,   // drawn on canvas (calls, imports, etc.)
    layout_edges: Vec<GraphEdge>,        // structural edges (Contains) used only for force layout
    pub node_index: HashMap<String, usize>,

    // UI state
    pub search: String,
    pub filters: Filters,
    pub selected_id: Option<String>,
    pub callers: Vec<NodeEdgeRef>,
    pub callees: Vec<NodeEdgeRef>,
    pub viewport: Viewport,

    // Layout
    layout_temp: f32,
    pub layout_running: bool,

    // Error display
    pub error: Option<String>,

    // MCP server
    mcp_handle: Option<McpHandle>,

    // Sync
    pub syncing: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            project_path: PathBuf::from("."),
            all_nodes: Vec::new(),
            all_edges: Vec::new(),
            unused_no_inbound: HashSet::new(),
            unused_unexported: HashSet::new(),
            orphan_files: HashSet::new(),
            dead_routes: HashSet::new(),
            visible_nodes: Vec::new(),
            visible_edges: Vec::new(),
            layout_edges: Vec::new(),
            node_index: HashMap::new(),
            search: String::new(),
            filters: Filters::default(),
            selected_id: None,
            callers: Vec::new(),
            callees: Vec::new(),
            viewport: Viewport::default(),
            layout_temp: CANVAS_W / 10.0,
            layout_running: false,
            error: None,
            mcp_handle: None,
            syncing: false,
        }
    }
}

impl App {
    pub fn new(path: PathBuf) -> (Self, Task<Message>) {
        let mut app = App { project_path: path.clone(), ..App::default() };

        match load_graph(&path) {
            Ok(data) => {
                app.all_nodes = data.nodes;
                app.all_edges = data.edges;
                app.unused_no_inbound = data.unused_no_inbound;
                app.unused_unexported = data.unused_unexported;
                app.orphan_files = data.orphan_files;
                app.dead_routes = data.dead_routes;
                app.rebuild_visible();
                app.fit_viewport(900.0, 650.0);
            }
            Err(e) => {
                app.error = Some(format!(
                    "Failed to load CodeGraph database at {:?}: {}",
                    path, e
                ));
            }
        }

        (app, Task::none())
    }

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::SearchChanged(q) => {
                self.search = q;
                self.rebuild_visible();
            }

            Message::FilterChanged(FilterKind::ToggleKind(kind)) => {
                if self.filters.hidden_kinds.contains(&kind) {
                    self.filters.hidden_kinds.remove(&kind);
                } else {
                    self.filters.hidden_kinds.insert(kind);
                }
                self.rebuild_visible();
            }

            Message::UnusedFilterChanged(kind) => {
                match kind {
                    UnusedFilterKind::NoInbound => {
                        self.filters.unused_no_inbound = !self.filters.unused_no_inbound;
                    }
                    UnusedFilterKind::Unexported => {
                        self.filters.unused_unexported = !self.filters.unused_unexported;
                    }
                    UnusedFilterKind::OrphanFiles => {
                        self.filters.unused_orphan_files = !self.filters.unused_orphan_files;
                    }
                    UnusedFilterKind::DeadRoutes => {
                        self.filters.unused_dead_routes = !self.filters.unused_dead_routes;
                    }
                }
                self.rebuild_visible();
            }

            Message::NodeSelected(id) => {
                if self.selected_id.as_deref() == Some(&id) {
                    return Task::none();
                }
                self.selected_id = Some(id.clone());
                self.load_detail(&id);
                // Pan to bring selected node into view
                self.pan_to_node(&id);
            }

            Message::NodeDeselected => {
                self.selected_id = None;
                self.callers.clear();
                self.callees.clear();
            }

            Message::NodeMoved { id, x, y } => {
                if let Some(&i) = self.node_index.get(&id) {
                    self.visible_nodes[i].x = x;
                    self.visible_nodes[i].y = y;
                    self.visible_nodes[i].pinned = true;
                }
            }

            Message::Pan { dx, dy } => {
                self.viewport.pan_x += dx;
                self.viewport.pan_y += dy;
            }

            Message::Zoom { factor, center_x, center_y } => {
                let old_zoom = self.viewport.zoom;
                let new_zoom = (old_zoom * factor).clamp(0.05, 5.0);
                // Keep world point under cursor fixed
                self.viewport.pan_x = center_x - (center_x - self.viewport.pan_x) * (new_zoom / old_zoom);
                self.viewport.pan_y = center_y - (center_y - self.viewport.pan_y) * (new_zoom / old_zoom);
                self.viewport.zoom = new_zoom;
            }

            Message::LayoutTick => {
                if self.layout_temp > 0.5 {
                    layout::tick(
                        &mut self.visible_nodes,
                        &self.visible_edges,
                        &self.layout_edges,
                        &self.node_index,
                        self.layout_temp,
                    );
                    self.layout_temp *= 0.97;
                } else {
                    self.layout_running = false;
                    self.fit_viewport(900.0, 650.0);
                }
            }

            Message::Sync => {
                if self.syncing {
                    return Task::none();
                }
                self.syncing = true;
                let path = self.project_path.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let config = load_config(&path)
                                .unwrap_or_else(|_| create_default_config(&path));
                            sync_project(&path, &config)
                                .map(|s| {
                                    format!(
                                        "{} files re-indexed, {} nodes updated",
                                        s.files_reindexed, s.nodes_updated
                                    )
                                })
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    Message::SyncComplete,
                );
            }

            Message::SyncComplete(result) => {
                self.syncing = false;
                match result {
                    Err(e) => {
                        self.error = Some(format!("Sync failed: {e}"));
                    }
                    Ok(_) => {
                        let path = self.project_path.clone();
                        match load_graph(&path) {
                            Ok(data) => {
                                self.all_nodes = data.nodes;
                                self.all_edges = data.edges;
                                self.unused_no_inbound = data.unused_no_inbound;
                                self.unused_unexported = data.unused_unexported;
                                self.orphan_files = data.orphan_files;
                                self.dead_routes = data.dead_routes;
                                self.rebuild_visible();
                                self.fit_viewport(900.0, 650.0);
                            }
                            Err(e) => {
                                self.error = Some(format!("Reload after sync failed: {e}"));
                            }
                        }
                    }
                }
            }

            Message::ToggleMcp => {
                if let Some(handle) = self.mcp_handle.take() {
                    handle.stop();
                } else {
                    match mcp_server::start(self.project_path.clone(), 4242) {
                        Ok(handle) => {
                            self.mcp_handle = Some(handle);
                        }
                        Err(e) => {
                            self.error = Some(format!("Failed to start MCP server: {e}"));
                        }
                    }
                }
            }
        }

        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        if let Some(ref err) = self.error {
            return container(
                column![
                    text("CodeGraph UI").size(24).color(theme::TEXT_PRIMARY),
                    text(err).size(14).color(iced::Color::from_rgb8(0xFF, 0x66, 0x66)),
                ]
                .spacing(16)
                .padding(40),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(theme::BACKGROUND)),
                ..Default::default()
            })
            .into();
        }

        let sidebar = sidebar::view(sidebar::SidebarState {
            search: &self.search,
            hidden_kinds: &self.filters.hidden_kinds,
            unused_no_inbound: self.filters.unused_no_inbound,
            unused_unexported: self.filters.unused_unexported,
            unused_orphan_files: self.filters.unused_orphan_files,
            unused_dead_routes: self.filters.unused_dead_routes,
            node_count: self.visible_nodes.len(),
            edge_count: self.visible_edges.len(),
        });

        let graph_canvas = canvas::build(
            &self.visible_nodes,
            &self.visible_edges,
            self.selected_id.as_deref(),
            &self.viewport,
            &self.node_index,
        );

        let main_area: Element<Message> = if let Some(ref id) = self.selected_id {
            if let Some(node) = self.all_nodes.iter().find(|n| &n.id == id) {
                let detail_panel = detail::view(detail::DetailState {
                    node,
                    callers: &self.callers,
                    callees: &self.callees,
                });
                row![graph_canvas, detail_panel].into()
            } else {
                graph_canvas.into()
            }
        } else {
            graph_canvas.into()
        };

        let sync_label = if self.syncing { "Syncing..." } else { "Sync" };
        let sync_color = if self.syncing {
            iced::Color::from_rgb8(0xFF, 0xD1, 0x66)
        } else {
            theme::TEXT_MUTED
        };
        let mut sync_btn = button(text(sync_label).size(12).color(sync_color))
            .style(header_btn_style)
            .padding([4, 10]);
        if !self.syncing {
            sync_btn = sync_btn.on_press(Message::Sync);
        }

        let mcp_label = match &self.mcp_handle {
            Some(h) => format!("MCP  :{}  Stop", h.port),
            None => "MCP  Start".to_string(),
        };
        let mcp_btn = button(
            text(mcp_label)
                .size(12)
                .color(match &self.mcp_handle {
                    Some(_) => iced::Color::from_rgb8(0x4D, 0xED, 0xA5),
                    None => theme::TEXT_MUTED,
                }),
        )
        .on_press(Message::ToggleMcp)
        .style(header_btn_style)
        .padding([4, 10]);

        let header = container(
            row![
                iced::widget::Space::with_width(Length::Fill),
                sync_btn,
                mcp_btn
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .padding([4, 8]),
        )
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(theme::SIDEBAR_BG)),
            ..Default::default()
        });

        let content = column![header, main_area].width(Length::Fill).height(Length::Fill);

        row![sidebar, content]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.layout_running {
            iced::time::every(Duration::from_millis(16)).map(|_| Message::LayoutTick)
        } else {
            Subscription::none()
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn rebuild_visible(&mut self) {
        let search_lower = self.search.to_lowercase();

        // Build the active unused ID set (union of selected unused filters)
        let active_unused: Option<HashSet<&str>> = if self.filters.any_unused() {
            let mut ids: HashSet<&str> = HashSet::new();
            if self.filters.unused_no_inbound {
                ids.extend(self.unused_no_inbound.iter().map(String::as_str));
            }
            if self.filters.unused_unexported {
                ids.extend(self.unused_unexported.iter().map(String::as_str));
            }
            if self.filters.unused_orphan_files {
                ids.extend(self.orphan_files.iter().map(String::as_str));
            }
            if self.filters.unused_dead_routes {
                ids.extend(self.dead_routes.iter().map(String::as_str));
            }
            Some(ids)
        } else {
            None
        };

        // Collect old positions to preserve them
        let old_pos: HashMap<String, (f32, f32)> = self
            .visible_nodes
            .iter()
            .map(|n| (n.node.id.clone(), (n.x, n.y)))
            .collect();

        // Filter nodes
        let filtered: Vec<&Node> = self
            .all_nodes
            .iter()
            .filter(|n| {
                if self.filters.hidden_kinds.contains(&n.kind) {
                    return false;
                }
                if let Some(ref ids) = active_unused {
                    if !ids.contains(n.id.as_str()) {
                        return false;
                    }
                }
                if !search_lower.is_empty() {
                    let matches = n.name.to_lowercase().contains(&search_lower)
                        || n.qualified_name.to_lowercase().contains(&search_lower)
                        || n.file_path.to_lowercase().contains(&search_lower);
                    if !matches {
                        return false;
                    }
                }
                true
            })
            .take(MAX_VISIBLE_NODES)
            .collect();

        let n = filtered.len();

        // All unused IDs for highlight detection
        let all_unused: HashSet<&str> = {
            let mut s = HashSet::new();
            s.extend(self.unused_no_inbound.iter().map(String::as_str));
            s.extend(self.unused_unexported.iter().map(String::as_str));
            s.extend(self.orphan_files.iter().map(String::as_str));
            s.extend(self.dead_routes.iter().map(String::as_str));
            s
        };

        // Assign positions (keep old, circle-init for new)
        let initial = layout::initial_positions(n);

        self.visible_nodes = filtered
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let (x, y) = old_pos.get(&node.id).copied().unwrap_or(initial[i]);
                GraphNode {
                    node: (*node).clone(),
                    x,
                    y,
                    is_unused: all_unused.contains(node.id.as_str()),
                    pinned: false,
                }
            })
            .collect();

        // Build lookup index
        self.node_index = self
            .visible_nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.node.id.clone(), i))
            .collect();

        let visible_ids: HashSet<&str> =
            self.visible_nodes.iter().map(|n| n.node.id.as_str()).collect();

        // Split edges: draw_edges (calls/imports – shown on canvas) and
        // layout_edges (Contains – used only to cluster nodes hierarchically).
        let mut draw_edges = Vec::new();
        let mut struct_edges = Vec::new();
        for e in &self.all_edges {
            if e.source == e.target {
                continue;
            }
            if !visible_ids.contains(e.source.as_str())
                || !visible_ids.contains(e.target.as_str())
            {
                continue;
            }
            if e.kind == EdgeKind::Contains {
                struct_edges.push(GraphEdge { edge: e.clone() });
            } else {
                draw_edges.push(GraphEdge { edge: e.clone() });
            }
        }
        self.visible_edges = draw_edges;
        self.layout_edges = struct_edges;

        // Reset layout
        self.layout_temp = CANVAS_W / 10.0;
        self.layout_running = n > 1;
    }

    fn load_detail(&mut self, node_id: &str) {
        // Load callers and callees from DB (synchronous — fast for single node)
        let Ok(qs) = QueryService::open(&self.project_path) else { return };
        let gs = GraphService::new(&qs);

        self.callers = gs.get_callers(node_id, 2).unwrap_or_default();
        self.callees = gs.get_callees(node_id, 2).unwrap_or_default();
    }

    fn fit_viewport(&mut self, canvas_w: f32, canvas_h: f32) {
        if self.visible_nodes.is_empty() {
            return;
        }
        let padding = 60.0f32;
        let min_x = self.visible_nodes.iter().map(|n| n.x).fold(f32::MAX, f32::min);
        let max_x = self.visible_nodes.iter().map(|n| n.x).fold(f32::MIN, f32::max);
        let min_y = self.visible_nodes.iter().map(|n| n.y).fold(f32::MAX, f32::min);
        let max_y = self.visible_nodes.iter().map(|n| n.y).fold(f32::MIN, f32::max);
        let w = (max_x - min_x).max(1.0);
        let h = (max_y - min_y).max(1.0);
        let zoom = ((canvas_w - padding * 2.0) / w)
            .min((canvas_h - padding * 2.0) / h)
            .clamp(0.05, 3.0);
        self.viewport.zoom = zoom;
        self.viewport.pan_x = canvas_w / 2.0 - ((min_x + max_x) / 2.0) * zoom;
        self.viewport.pan_y = canvas_h / 2.0 - ((min_y + max_y) / 2.0) * zoom;
    }

    fn pan_to_node(&mut self, id: &str) {
        if let Some(&i) = self.node_index.get(id) {
            let node = &self.visible_nodes[i];
            // Bring node to viewport center (approximate — canvas size unknown here)
            let target_cx = 600.0f32; // rough canvas center x
            let target_cy = 400.0f32;
            self.viewport.pan_x = target_cx - node.x * self.viewport.zoom;
            self.viewport.pan_y = target_cy - node.y * self.viewport.zoom;
        }
    }
}

fn header_btn_style(_: &iced::Theme, _: button::Status) -> button::Style {
    button::Style {
        background: Some(iced::Background::Color(theme::SIDEBAR_BG)),
        border: iced::Border {
            color: iced::Color { a: 0.2, ..iced::Color::WHITE },
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}
