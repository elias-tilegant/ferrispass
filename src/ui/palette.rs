//! Design tokens that don't fit gpui-component's `Theme` color slots.
//!
//! The blue family lives in `Theme.primary` / `Theme.sidebar_accent` / `Theme.accent`;
//! the orange / muted-text / strong-border / soft-tinted backgrounds live here.

use gpui::Hsla;

pub const BG: Hsla = Hsla { h: 0.583333, s: 0.250000, l: 0.984314, a: 1.0 };
pub const PANEL: Hsla = Hsla { h: 0.000000, s: 0.000000, l: 1.000000, a: 1.0 };
pub const SIDEBAR: Hsla = Hsla { h: 0.611111, s: 0.142857, l: 0.958824, a: 1.0 };
pub const BORDER: Hsla = Hsla { h: 0.611111, s: 0.125000, l: 0.905882, a: 1.0 };
pub const BORDER_STRONG: Hsla = Hsla { h: 0.604167, s: 0.095238, l: 0.835294, a: 1.0 };
pub const TEXT: Hsla = Hsla { h: 0.604167, s: 0.125000, l: 0.125490, a: 1.0 };
pub const TEXT_MUTED: Hsla = Hsla { h: 0.611111, s: 0.089362, l: 0.460784, a: 1.0 };
pub const TEXT_FAINT: Hsla = Hsla { h: 0.605263, s: 0.106145, l: 0.649020, a: 1.0 };

pub const BLUE: Hsla = Hsla { h: 0.616580, s: 0.814346, l: 0.535294, a: 1.0 };
pub const BLUE_HOVER: Hsla = Hsla { h: 0.618182, s: 0.726872, l: 0.445098, a: 1.0 };
pub const BLUE_SOFT: Hsla = Hsla { h: 0.614035, s: 0.826087, l: 0.954902, a: 1.0 };
pub const BLUE_BORDER: Hsla = Hsla { h: 0.616071, s: 0.777778, l: 0.858824, a: 1.0 };

pub const ORANGE: Hsla = Hsla { h: 0.065412, s: 0.801724, l: 0.545098, a: 1.0 };
pub const ORANGE_DEEP: Hsla = Hsla { h: 0.066038, s: 0.753555, l: 0.413725, a: 1.0 };
pub const ORANGE_SOFT: Hsla = Hsla { h: 0.072464, s: 0.851852, l: 0.947059, a: 1.0 };
pub const ORANGE_BORDER: Hsla = Hsla { h: 0.080460, s: 0.813084, l: 0.790196, a: 1.0 };

pub const GREEN: Hsla = Hsla { h: 0.394799, s: 0.762162, l: 0.362745, a: 1.0 };
pub const GREEN_SOFT: Hsla = Hsla { h: 0.388889, s: 0.454545, l: 0.935294, a: 1.0 };
pub const GREEN_BORDER: Hsla = Hsla { h: 0.380952, s: 0.446809, l: 0.815686, a: 1.0 };

pub const RED: Hsla = Hsla { h: 0.000000, s: 0.722222, l: 0.505882, a: 1.0 };
pub const YELLOW: Hsla = Hsla { h: 0.089258, s: 0.946188, l: 0.437255, a: 1.0 };

pub const TRANSPARENT_OVERLAY: Hsla = Hsla { h: 0.604167, s: 0.125000, l: 0.125490, a: 0.32 };
