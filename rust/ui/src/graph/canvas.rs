use std::collections::HashMap;

use iced::{Color, Point, Rectangle, mouse};
use iced::widget::canvas::{
    self, Canvas, Frame, Geometry, Path, Stroke, Text as CanvasText,
};

use crate::app::Message;
use crate::graph::types::{GraphEdge, GraphNode, NODE_RADIUS, Viewport};
use crate::theme;

pub struct GraphCanvas<'a> {
    pub nodes: &'a [GraphNode],
    pub edges: &'a [GraphEdge],
    pub selected_id: Option<&'a str>,
    pub viewport: &'a Viewport,
    pub node_index: &'a HashMap<String, usize>,
}

#[derive(Default, Clone)]
pub struct CanvasState {
    dragging_node: Option<String>,
    panning: bool,
    last_cursor: Option<Point>,
    pressed_at: Option<Point>,
}

const DRAG_THRESHOLD_SQ: f32 = 25.0; // 5px²

impl<'a> canvas::Program<Message> for GraphCanvas<'a> {
    type State = CanvasState;

    fn update(
        &self,
        state: &mut CanvasState,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    state.pressed_at = Some(pos);
                    state.last_cursor = Some(pos);
                    if let Some(id) = self.find_node_at(pos) {
                        state.dragging_node = Some(id);
                    } else {
                        state.panning = true;
                    }
                    return (canvas::event::Status::Captured, None);
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let was_click = state.pressed_at
                    .zip(cursor.position_in(bounds))
                    .map(|(pressed, cur)| {
                        let dx = cur.x - pressed.x;
                        let dy = cur.y - pressed.y;
                        dx * dx + dy * dy < DRAG_THRESHOLD_SQ
                    })
                    .unwrap_or(false);

                let dragged_id = state.dragging_node.take();
                state.panning = false;
                state.last_cursor = None;
                state.pressed_at = None;

                if was_click {
                    let clicked_id = dragged_id.or_else(|| {
                        cursor.position_in(bounds).and_then(|p| self.find_node_at(p))
                    });
                    let msg = if let Some(id) = clicked_id {
                        Message::NodeSelected(id)
                    } else {
                        Message::NodeDeselected
                    };
                    return (canvas::event::Status::Captured, Some(msg));
                }
                return (canvas::event::Status::Captured, None);
            }

            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if let Some(ref id) = state.dragging_node {
                        let (wx, wy) = self.viewport.canvas_to_world(pos.x, pos.y);
                        let id = id.clone();
                        state.last_cursor = Some(pos);
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::NodeMoved { id, x: wx, y: wy }),
                        );
                    } else if state.panning {
                        if let Some(last) = state.last_cursor {
                            let dx = pos.x - last.x;
                            let dy = pos.y - last.y;
                            state.last_cursor = Some(pos);
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::Pan { dx, dy }),
                            );
                        }
                        state.last_cursor = Some(pos);
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let factor = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => {
                            if y > 0.0 { 1.1 } else { 0.9 }
                        }
                        mouse::ScrollDelta::Pixels { y, .. } => 1.0 + y * 0.005,
                    };
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::Zoom { factor, center_x: pos.x, center_y: pos.y }),
                    );
                }
            }

            _ => {}
        }

        (canvas::event::Status::Ignored, None)
    }

    fn draw(
        &self,
        _state: &CanvasState,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry<iced::Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());

        // Background
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BACKGROUND);

        // Draw edges behind nodes
        for edge in self.edges {
            self.draw_edge(&mut frame, edge, bounds);
        }

        // Determine hovered node
        let hover_id = cursor
            .position_in(bounds)
            .and_then(|p| self.find_node_at(p));

        // Draw nodes
        for node in self.nodes {
            let is_selected = self.selected_id.map_or(false, |id| id == node.node.id);
            let is_hovered =
                hover_id.as_deref().map_or(false, |id| id == node.node.id);
            self.draw_node(&mut frame, node, is_selected, is_hovered, bounds);
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &CanvasState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging_node.is_some() || state.panning {
            mouse::Interaction::Grabbing
        } else if cursor
            .position_in(bounds)
            .and_then(|p| self.find_node_at(p))
            .is_some()
        {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a> GraphCanvas<'a> {
    fn find_node_at(&self, canvas_pos: Point) -> Option<String> {
        let (wx, wy) = self.viewport.canvas_to_world(canvas_pos.x, canvas_pos.y);
        // Iterate in reverse so topmost-drawn node wins
        for node in self.nodes.iter().rev() {
            let dx = wx - node.x;
            let dy = wy - node.y;
            if dx * dx + dy * dy <= NODE_RADIUS * NODE_RADIUS {
                return Some(node.node.id.clone());
            }
        }
        None
    }

    fn draw_node(
        &self,
        frame: &mut Frame<iced::Renderer>,
        node: &GraphNode,
        selected: bool,
        hovered: bool,
        bounds: Rectangle,
    ) {
        let (cx, cy) = self.viewport.world_to_canvas(node.x, node.y);
        let r = (NODE_RADIUS * self.viewport.zoom).max(2.0);

        // Cull off-screen nodes
        if cx + r < 0.0
            || cx - r > bounds.width
            || cy + r < 0.0
            || cy - r > bounds.height
        {
            return;
        }

        let center = Point::new(cx, cy);
        let color = theme::node_color(&node.node.kind);
        let circle = Path::circle(center, r);

        // Glow for selected node
        if selected {
            let glow = Path::circle(center, r + 5.0);
            frame.fill(
                &glow,
                Color { a: 0.35, ..theme::ACCENT },
            );
        }

        frame.fill(&circle, color);

        // Outline
        if node.is_unused {
            frame.stroke(
                &circle,
                Stroke::default()
                    .with_color(theme::UNUSED_OUTLINE)
                    .with_width(2.5),
            );
        } else if selected {
            frame.stroke(
                &circle,
                Stroke::default().with_color(Color::WHITE).with_width(2.0),
            );
        } else if hovered {
            frame.stroke(
                &circle,
                Stroke::default()
                    .with_color(Color { a: 0.8, ..Color::WHITE })
                    .with_width(1.5),
            );
        }

        // Label — only show when zoom is large enough to read
        if r > 5.0 {
            let label = truncate(&node.node.name, 20);
            let font_size = (11.0 * self.viewport.zoom.max(0.6)).min(13.0);
            frame.fill_text(CanvasText {
                content: label,
                position: Point::new(cx, cy + r + 3.0),
                color: theme::LABEL_COLOR,
                size: font_size.into(),
                horizontal_alignment: iced::alignment::Horizontal::Center,
                vertical_alignment: iced::alignment::Vertical::Top,
                ..CanvasText::default()
            });
        }
    }

    fn draw_edge(
        &self,
        frame: &mut Frame<iced::Renderer>,
        edge: &GraphEdge,
        bounds: Rectangle,
    ) {
        let (Some(&si), Some(&ti)) = (
            self.node_index.get(&edge.edge.source),
            self.node_index.get(&edge.edge.target),
        ) else {
            return;
        };

        let src = &self.nodes[si];
        let tgt = &self.nodes[ti];

        let (sx, sy) = self.viewport.world_to_canvas(src.x, src.y);
        let (tx, ty) = self.viewport.world_to_canvas(tgt.x, tgt.y);

        // Broad cull: skip edges where both ends are off-screen
        let on_screen = |x: f32, y: f32| {
            x > -50.0 && x < bounds.width + 50.0 && y > -50.0 && y < bounds.height + 50.0
        };
        if !on_screen(sx, sy) && !on_screen(tx, ty) {
            return;
        }

        let r = NODE_RADIUS * self.viewport.zoom;
        let dx = tx - sx;
        let dy = ty - sy;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist < r * 2.0 {
            return; // Endpoints overlap — skip
        }

        let nx = dx / dist;
        let ny = dy / dist;
        let from = Point::new(sx + nx * r, sy + ny * r);
        let to = Point::new(tx - nx * r, ty - ny * r);

        let color = theme::edge_color(&edge.edge.kind);
        let line_width = (1.5 * self.viewport.zoom).clamp(0.8, 2.5);

        let line = Path::new(|b| {
            b.move_to(from);
            b.line_to(to);
        });
        frame.stroke(&line, Stroke::default().with_color(color).with_width(line_width));

        // Arrowhead at target
        let arrow_len = (7.0 * self.viewport.zoom).clamp(3.0, 10.0);
        let angle = dy.atan2(dx);
        let spread = std::f32::consts::FRAC_PI_6; // 30°
        let a1 = Point::new(
            to.x + arrow_len * (angle + std::f32::consts::PI - spread).cos(),
            to.y + arrow_len * (angle + std::f32::consts::PI - spread).sin(),
        );
        let a2 = Point::new(
            to.x + arrow_len * (angle + std::f32::consts::PI + spread).cos(),
            to.y + arrow_len * (angle + std::f32::consts::PI + spread).sin(),
        );
        let arrow = Path::new(|b| {
            b.move_to(to);
            b.line_to(a1);
            b.move_to(to);
            b.line_to(a2);
        });
        frame.stroke(
            &arrow,
            Stroke::default().with_color(color).with_width(line_width),
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let mut out: String = chars[..max - 1].iter().collect();
        out.push('…');
        out
    }
}

pub fn build<'a>(
    nodes: &'a [GraphNode],
    edges: &'a [GraphEdge],
    selected_id: Option<&'a str>,
    viewport: &'a Viewport,
    node_index: &'a HashMap<String, usize>,
) -> Canvas<GraphCanvas<'a>, Message> {
    Canvas::new(GraphCanvas { nodes, edges, selected_id, viewport, node_index })
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
}
