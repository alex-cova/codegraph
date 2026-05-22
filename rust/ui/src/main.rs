mod app;
mod db;
mod graph;
mod mcp_server;
mod panels;
mod theme;

use std::path::PathBuf;

fn main() -> iced::Result {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    iced::application("CodeGraph", app::App::update, app::App::view)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .run_with(move || app::App::new(path))
}
