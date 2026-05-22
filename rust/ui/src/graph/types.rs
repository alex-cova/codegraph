use codegraph_rs::types::{Edge, Node};

pub const NODE_RADIUS: f32 = 18.0;
pub const CANVAS_W: f32 = 2400.0;
pub const CANVAS_H: f32 = 1800.0;
pub const MAX_VISIBLE_NODES: usize = 800;

#[derive(Debug, Clone)]
pub struct GraphNode {
    pub node: Node,
    pub x: f32,
    pub y: f32,
    pub is_unused: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone)]
pub struct GraphEdge {
    pub edge: Edge,
}

#[derive(Debug, Clone)]
pub struct Viewport {
    pub pan_x: f32,
    pub pan_y: f32,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self { pan_x: 0.0, pan_y: 0.0, zoom: 0.4 }
    }
}

impl Viewport {
    pub fn world_to_canvas(&self, wx: f32, wy: f32) -> (f32, f32) {
        (wx * self.zoom + self.pan_x, wy * self.zoom + self.pan_y)
    }

    pub fn canvas_to_world(&self, cx: f32, cy: f32) -> (f32, f32) {
        ((cx - self.pan_x) / self.zoom, (cy - self.pan_y) / self.zoom)
    }
}
