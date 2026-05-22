use codegraph_rs::types::NodeKind;
use iced::widget::{
    checkbox, column, container, scrollable, text, text_input, Column, Space,
};
use iced::{Element, Length};

use crate::app::{FilterKind, Message, UnusedFilterKind};
use crate::theme;

pub struct SidebarState<'a> {
    pub search: &'a str,
    pub hidden_kinds: &'a std::collections::HashSet<NodeKind>,
    pub unused_no_inbound: bool,
    pub unused_unexported: bool,
    pub unused_orphan_files: bool,
    pub unused_dead_routes: bool,
    pub node_count: usize,
    pub edge_count: usize,
}

pub fn view<'a>(state: SidebarState<'a>) -> Element<'a, Message> {
    let search_bar = text_input("Search symbols...", state.search)
        .on_input(Message::SearchChanged)
        .padding(10)
        .size(14);

    let stats = text(format!(
        "{} nodes · {} edges",
        state.node_count, state.edge_count
    ))
    .size(11)
    .color(theme::TEXT_MUTED);

    let kind_section = kind_filters(state.hidden_kinds);
    let unused_section = unused_filters(
        state.unused_no_inbound,
        state.unused_unexported,
        state.unused_orphan_files,
        state.unused_dead_routes,
    );

    let content = column![
        search_bar,
        Space::with_height(6),
        stats,
        Space::with_height(14),
        section_header("Node Kinds"),
        Space::with_height(6),
        kind_section,
        Space::with_height(14),
        section_header("Unused Code"),
        Space::with_height(6),
        unused_section,
    ]
    .spacing(2)
    .padding(12);

    container(scrollable(content))
        .width(Length::Fixed(230.0))
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::SIDEBAR_BG)),
            ..container::Style::default()
        })
        .into()
}

fn section_header(label: &str) -> Element<'_, Message> {
    text(label).size(11).color(theme::SECTION_HEADER).into()
}

fn kind_toggle<'a>(
    label: &'a str,
    kind: NodeKind,
    hidden: &std::collections::HashSet<NodeKind>,
) -> Element<'a, Message> {
    let checked = !hidden.contains(&kind);
    checkbox(label, checked)
        .on_toggle(move |_| Message::FilterChanged(FilterKind::ToggleKind(kind.clone())))
        .size(14)
        .text_size(13)
        .into()
}

fn kind_filters<'a>(
    hidden: &'a std::collections::HashSet<NodeKind>,
) -> Element<'a, Message> {
    Column::new()
        .push(kind_toggle("Files", NodeKind::File, hidden))
        .push(kind_toggle("Classes & Structs", NodeKind::Class, hidden))
        .push(kind_toggle("Interfaces & Traits", NodeKind::Interface, hidden))
        .push(kind_toggle("Functions", NodeKind::Function, hidden))
        .push(kind_toggle("Methods", NodeKind::Method, hidden))
        .push(kind_toggle("Routes", NodeKind::Route, hidden))
        .push(kind_toggle("Components", NodeKind::Component, hidden))
        .push(kind_toggle("Enums", NodeKind::Enum, hidden))
        .push(kind_toggle("Variables", NodeKind::Variable, hidden))
        .spacing(4)
        .into()
}

fn unused_filters<'a>(
    no_inbound: bool,
    unexported: bool,
    orphan_files: bool,
    dead_routes: bool,
) -> Element<'a, Message> {
    Column::new()
        .push(
            checkbox("No inbound edges", no_inbound).on_toggle(|_| {
                Message::UnusedFilterChanged(UnusedFilterKind::NoInbound)
            }).size(14).text_size(13),
        )
        .push(
            checkbox("Unexported + unreferenced", unexported).on_toggle(|_| {
                Message::UnusedFilterChanged(UnusedFilterKind::Unexported)
            }).size(14).text_size(13),
        )
        .push(
            checkbox("Orphan files", orphan_files).on_toggle(|_| {
                Message::UnusedFilterChanged(UnusedFilterKind::OrphanFiles)
            }).size(14).text_size(13),
        )
        .push(
            checkbox("Dead routes", dead_routes).on_toggle(|_| {
                Message::UnusedFilterChanged(UnusedFilterKind::DeadRoutes)
            }).size(14).text_size(13),
        )
        .spacing(4)
        .into()
}
