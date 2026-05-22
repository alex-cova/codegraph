use std::collections::HashMap;

use crate::graph::types::{GraphEdge, GraphNode, CANVAS_H, CANVAS_W};

/// `edges` = edges to draw; `layout_edges` = structural edges (Contains etc.)
/// used only for force computation, not drawn.
pub fn tick(
    nodes: &mut [GraphNode],
    edges: &[GraphEdge],
    layout_edges: &[GraphEdge],
    node_index: &HashMap<String, usize>,
    temperature: f32,
) {
    let n = nodes.len();
    if n < 2 {
        return;
    }

    let k = ((CANVAS_W * CANVAS_H) / n as f32).sqrt();
    let mut fx = vec![0.0f32; n];
    let mut fy = vec![0.0f32; n];

    // Repulsion between every pair of nodes
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = nodes[j].x - nodes[i].x;
            let dy = nodes[j].y - nodes[i].y;
            let dist = (dx * dx + dy * dy).sqrt().max(1.0);
            let force = k * k / dist;
            let dfx = force * dx / dist;
            let dfy = force * dy / dist;
            fx[i] -= dfx;
            fy[i] -= dfy;
            fx[j] += dfx;
            fy[j] += dfy;
        }
    }

    // Attraction along all graph edges (draw + structural)
    for edge in edges.iter().chain(layout_edges.iter()) {
        let (Some(&i), Some(&j)) = (
            node_index.get(&edge.edge.source),
            node_index.get(&edge.edge.target),
        ) else {
            continue;
        };
        if i == j {
            continue;
        }
        let dx = nodes[j].x - nodes[i].x;
        let dy = nodes[j].y - nodes[i].y;
        let dist = (dx * dx + dy * dy).sqrt().max(1.0);
        let force = dist * dist / k;
        let dfx = force * dx / dist;
        let dfy = force * dy / dist;
        fx[i] += dfx;
        fy[i] += dfy;
        fx[j] -= dfx;
        fy[j] -= dfy;
    }

    // Apply forces clamped by temperature
    for i in 0..n {
        if nodes[i].pinned {
            continue;
        }
        let force_len = (fx[i] * fx[i] + fy[i] * fy[i]).sqrt().max(0.001);
        let clamped = force_len.min(temperature);
        nodes[i].x += fx[i] / force_len * clamped;
        nodes[i].y += fy[i] / force_len * clamped;
        nodes[i].x = nodes[i].x.clamp(50.0, CANVAS_W - 50.0);
        nodes[i].y = nodes[i].y.clamp(50.0, CANVAS_H - 50.0);
    }
}

pub fn initial_positions(n: usize) -> Vec<(f32, f32)> {
    let cx = CANVAS_W / 2.0;
    let cy = CANVAS_H / 2.0;
    let r = (CANVAS_W.min(CANVAS_H) / 3.0).max(200.0);
    (0..n)
        .map(|i| {
            let angle = 2.0 * std::f32::consts::PI * i as f32 / n.max(1) as f32;
            (cx + r * angle.cos(), cy + r * angle.sin())
        })
        .collect()
}
