use codegraph_rs::types::{EdgeKind, NodeKind};
use iced::Color;

pub const BACKGROUND: Color = Color { r: 0.071, g: 0.071, b: 0.094, a: 1.0 };
pub const SIDEBAR_BG: Color = Color { r: 0.09, g: 0.09, b: 0.118, a: 1.0 };
pub const SECTION_HEADER: Color = Color { r: 0.55, g: 0.55, b: 0.65, a: 1.0 };
pub const LABEL_COLOR: Color = Color { r: 0.78, g: 0.78, b: 0.85, a: 1.0 };
pub const ACCENT: Color = Color { r: 0.29, g: 0.62, b: 1.0, a: 1.0 };
pub const UNUSED_OUTLINE: Color = Color { r: 1.0, g: 0.76, b: 0.02, a: 1.0 };
pub const TEXT_PRIMARY: Color = Color { r: 0.9, g: 0.9, b: 0.95, a: 1.0 };
pub const TEXT_MUTED: Color = Color { r: 0.55, g: 0.55, b: 0.65, a: 1.0 };

pub fn node_color(kind: &NodeKind) -> Color {
    match kind {
        NodeKind::File => Color::from_rgb8(0x4A, 0x9E, 0xFF),
        NodeKind::Module | NodeKind::Namespace => Color::from_rgb8(0x5B, 0xB8, 0xFF),
        NodeKind::Class | NodeKind::Struct => Color::from_rgb8(0xFF, 0x8C, 0x42),
        NodeKind::Interface | NodeKind::Trait | NodeKind::Protocol => {
            Color::from_rgb8(0xC7, 0x7D, 0xFF)
        }
        NodeKind::Function | NodeKind::Method => Color::from_rgb8(0x4D, 0xED, 0xA5),
        NodeKind::Route => Color::from_rgb8(0xFF, 0x4D, 0x4D),
        NodeKind::Component => Color::from_rgb8(0xFF, 0xD1, 0x66),
        NodeKind::Enum | NodeKind::EnumMember => Color::from_rgb8(0xFF, 0xA0, 0x6A),
        NodeKind::TypeAlias => Color::from_rgb8(0xD0, 0xA0, 0xFF),
        NodeKind::Constant => Color::from_rgb8(0xFF, 0xE0, 0x80),
        NodeKind::Variable | NodeKind::Field | NodeKind::Property => {
            Color::from_rgb8(0x9E, 0x9E, 0x9E)
        }
        NodeKind::Import | NodeKind::Export => Color::from_rgb8(0x60, 0x60, 0x70),
        NodeKind::Parameter => Color::from_rgb8(0x70, 0x70, 0x80),
    }
}

pub fn edge_color(kind: &EdgeKind) -> Color {
    match kind {
        EdgeKind::Calls => Color { r: 0.30, g: 0.93, b: 0.65, a: 0.75 },
        EdgeKind::Imports => Color { r: 0.29, g: 0.62, b: 1.00, a: 0.70 },
        EdgeKind::Extends => Color { r: 1.00, g: 0.55, b: 0.26, a: 0.75 },
        EdgeKind::Implements => Color { r: 0.78, g: 0.49, b: 1.00, a: 0.70 },
        EdgeKind::References => Color { r: 0.75, g: 0.75, b: 0.85, a: 0.55 },
        EdgeKind::Contains => Color { r: 0.40, g: 0.40, b: 0.50, a: 0.20 },
        _ => Color { r: 0.65, g: 0.65, b: 0.70, a: 0.55 },
    }
}
