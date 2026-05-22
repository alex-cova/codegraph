use codegraph_rs::types::{Node, NodeEdgeRef};
use iced::widget::{button, container, horizontal_rule, scrollable, text, Column, Space};
use iced::{Element, Length};

use crate::app::Message;
use crate::theme;

pub struct DetailState<'a> {
    pub node: &'a Node,
    pub callers: &'a [NodeEdgeRef],
    pub callees: &'a [NodeEdgeRef],
}

pub fn view<'a>(state: DetailState<'a>) -> Element<'a, Message> {
    let node = state.node;

    let kind_str = format!("{:?}", node.kind).to_lowercase();
    let lang_str = format!("{:?}", node.language).to_lowercase();
    let exported = node
        .is_exported
        .map(|e| if e { "yes" } else { "no" })
        .unwrap_or("—");

    let file_short = shorten_path(&node.file_path, 32);
    let lines = if node.start_line == node.end_line {
        format!(":{}", node.start_line)
    } else {
        format!(":{}-{}", node.start_line, node.end_line)
    };

    let mut col = Column::new()
        .push(text(&node.name).size(15).color(theme::TEXT_PRIMARY))
        .push(Space::with_height(4))
        .push(text(format!("{} · {}", kind_str, lang_str)).size(11).color(theme::TEXT_MUTED))
        .push(Space::with_height(2))
        .push(text(format!("{}{}", file_short, lines)).size(11).color(theme::TEXT_MUTED))
        .push(Space::with_height(2))
        .push(text(format!("exported: {}", exported)).size(11).color(theme::TEXT_MUTED));

    if let Some(ref sig) = node.signature {
        col = col
            .push(Space::with_height(8))
            .push(horizontal_rule(1))
            .push(Space::with_height(6))
            .push(text("Signature").size(11).color(theme::SECTION_HEADER))
            .push(Space::with_height(3))
            .push(text(truncate(sig, 120)).size(11).color(theme::TEXT_PRIMARY).font(iced::Font::MONOSPACE));
    }

    if !state.callers.is_empty() {
        col = col
            .push(Space::with_height(8))
            .push(horizontal_rule(1))
            .push(Space::with_height(6))
            .push(
                text(format!("Callers ({})", state.callers.len()))
                    .size(11)
                    .color(theme::SECTION_HEADER),
            )
            .push(Space::with_height(3));
        for nr in state.callers.iter().take(10) {
            let caller = &nr.node;
            let label = format!(
                "→ {}  {}:{}",
                truncate(&caller.name, 22),
                shorten_path(&caller.file_path, 20),
                caller.start_line
            );
            let id = caller.id.clone();
            col = col.push(
                button(text(label).size(11).color(theme::ACCENT))
                    .style(button::text)
                    .on_press(Message::NodeSelected(id))
                    .padding(0),
            );
        }
        if state.callers.len() > 10 {
            col = col.push(
                text(format!("  …and {} more", state.callers.len() - 10))
                    .size(10)
                    .color(theme::TEXT_MUTED),
            );
        }
    }

    if !state.callees.is_empty() {
        col = col
            .push(Space::with_height(8))
            .push(horizontal_rule(1))
            .push(Space::with_height(6))
            .push(
                text(format!("Callees ({})", state.callees.len()))
                    .size(11)
                    .color(theme::SECTION_HEADER),
            )
            .push(Space::with_height(3));
        for nr in state.callees.iter().take(10) {
            let callee = &nr.node;
            let label = format!(
                "→ {}  {}:{}",
                truncate(&callee.name, 22),
                shorten_path(&callee.file_path, 20),
                callee.start_line
            );
            let id = callee.id.clone();
            col = col.push(
                button(text(label).size(11).color(theme::ACCENT))
                    .style(button::text)
                    .on_press(Message::NodeSelected(id))
                    .padding(0),
            );
        }
        if state.callees.len() > 10 {
            col = col.push(
                text(format!("  …and {} more", state.callees.len() - 10))
                    .size(10)
                    .color(theme::TEXT_MUTED),
            );
        }
    }

    container(scrollable(col.spacing(2).padding(12)))
        .width(Length::Fixed(260.0))
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::SIDEBAR_BG)),
            ..container::Style::default()
        })
        .into()
}

fn shorten_path(path: &str, max_chars: usize) -> String {
    let s = path.replace('\\', "/");
    if s.len() <= max_chars {
        return s;
    }
    // Keep the last segments that fit
    let parts: Vec<&str> = s.split('/').collect();
    let mut acc = String::new();
    for part in parts.iter().rev() {
        if acc.is_empty() {
            acc = part.to_string();
        } else {
            let candidate = format!("{}/{}", part, acc);
            if candidate.len() > max_chars {
                acc = format!("…/{}", acc);
                break;
            }
            acc = candidate;
        }
    }
    acc
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
