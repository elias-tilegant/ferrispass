//! Design tokens that don't fit gpui-component's `Theme` color slots.
//!
//! Tokens are accessed as runtime functions (`palette::panel()` etc.) rather than
//! `const`s, because they switch between [`LIGHT`] and [`DARK`] when the theme
//! mode changes. The active palette is selected by an atomic flag set from
//! [`crate::ui::theme::apply`]; switching is O(1) and lock-free.

use gpui::Hsla;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub panel: Hsla,
    pub sidebar: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub blue: Hsla,
    pub blue_hover: Hsla,
    pub blue_soft: Hsla,
    pub blue_border: Hsla,
    pub orange: Hsla,
    pub orange_deep: Hsla,
    pub orange_soft: Hsla,
    pub orange_border: Hsla,
    pub green: Hsla,
    pub green_soft: Hsla,
    pub green_border: Hsla,
    pub red: Hsla,
    pub yellow: Hsla,
    pub transparent_overlay: Hsla,
}

pub const LIGHT: Palette = Palette {
    bg: Hsla {
        h: 0.583333,
        s: 0.250000,
        l: 0.984314,
        a: 1.0,
    },
    panel: Hsla {
        h: 0.000000,
        s: 0.000000,
        l: 1.000000,
        a: 1.0,
    },
    sidebar: Hsla {
        h: 0.611111,
        s: 0.142857,
        l: 0.958824,
        a: 1.0,
    },
    border: Hsla {
        h: 0.611111,
        s: 0.125000,
        l: 0.905882,
        a: 1.0,
    },
    border_strong: Hsla {
        h: 0.604167,
        s: 0.095238,
        l: 0.835294,
        a: 1.0,
    },
    text: Hsla {
        h: 0.604167,
        s: 0.125000,
        l: 0.125490,
        a: 1.0,
    },
    text_muted: Hsla {
        h: 0.611111,
        s: 0.089362,
        l: 0.460784,
        a: 1.0,
    },
    text_faint: Hsla {
        h: 0.605263,
        s: 0.106145,
        l: 0.649020,
        a: 1.0,
    },
    blue: Hsla {
        h: 0.616580,
        s: 0.814346,
        l: 0.535294,
        a: 1.0,
    },
    blue_hover: Hsla {
        h: 0.618182,
        s: 0.726872,
        l: 0.445098,
        a: 1.0,
    },
    blue_soft: Hsla {
        h: 0.614035,
        s: 0.826087,
        l: 0.954902,
        a: 1.0,
    },
    blue_border: Hsla {
        h: 0.616071,
        s: 0.777778,
        l: 0.858824,
        a: 1.0,
    },
    orange: Hsla {
        h: 0.065412,
        s: 0.801724,
        l: 0.545098,
        a: 1.0,
    },
    orange_deep: Hsla {
        h: 0.066038,
        s: 0.753555,
        l: 0.413725,
        a: 1.0,
    },
    orange_soft: Hsla {
        h: 0.072464,
        s: 0.851852,
        l: 0.947059,
        a: 1.0,
    },
    orange_border: Hsla {
        h: 0.080460,
        s: 0.813084,
        l: 0.790196,
        a: 1.0,
    },
    green: Hsla {
        h: 0.394799,
        s: 0.762162,
        l: 0.362745,
        a: 1.0,
    },
    green_soft: Hsla {
        h: 0.388889,
        s: 0.454545,
        l: 0.935294,
        a: 1.0,
    },
    green_border: Hsla {
        h: 0.380952,
        s: 0.446809,
        l: 0.815686,
        a: 1.0,
    },
    red: Hsla {
        h: 0.000000,
        s: 0.722222,
        l: 0.505882,
        a: 1.0,
    },
    yellow: Hsla {
        h: 0.089258,
        s: 0.946188,
        l: 0.437255,
        a: 1.0,
    },
    transparent_overlay: Hsla {
        h: 0.604167,
        s: 0.125000,
        l: 0.125490,
        a: 0.32,
    },
};

pub const DARK: Palette = Palette {
    bg: Hsla {
        h: 0.611111,
        s: 0.103448,
        l: 0.056863,
        a: 1.0,
    },
    panel: Hsla {
        h: 0.611111,
        s: 0.120000,
        l: 0.098039,
        a: 1.0,
    },
    sidebar: Hsla {
        h: 0.611111,
        s: 0.157895,
        l: 0.074510,
        a: 1.0,
    },
    border: Hsla {
        h: 0.616667,
        s: 0.116279,
        l: 0.168627,
        a: 1.0,
    },
    border_strong: Hsla {
        h: 0.616667,
        s: 0.079365,
        l: 0.247059,
        a: 1.0,
    },
    text: Hsla {
        h: 0.611111,
        s: 0.125000,
        l: 0.905882,
        a: 1.0,
    },
    text_muted: Hsla {
        h: 0.595238,
        s: 0.074468,
        l: 0.631373,
        a: 1.0,
    },
    text_faint: Hsla {
        h: 0.600000,
        s: 0.066079,
        l: 0.445098,
        a: 1.0,
    },
    blue: Hsla {
        h: 0.616667,
        s: 0.833333,
        l: 0.623529,
        a: 1.0,
    },
    blue_hover: Hsla {
        h: 0.618056,
        s: 1.000000,
        l: 0.717647,
        a: 1.0,
    },
    blue_soft: Hsla {
        h: 0.625000,
        s: 0.391304,
        l: 0.180392,
        a: 1.0,
    },
    blue_border: Hsla {
        h: 0.617816,
        s: 0.397260,
        l: 0.286275,
        a: 1.0,
    },
    orange: Hsla {
        h: 0.067511,
        s: 0.822917,
        l: 0.623529,
        a: 1.0,
    },
    orange_deep: Hsla {
        h: 0.070946,
        s: 0.606557,
        l: 0.478431,
        a: 1.0,
    },
    orange_soft: Hsla {
        h: 0.076389,
        s: 0.400000,
        l: 0.117647,
        a: 1.0,
    },
    orange_border: Hsla {
        h: 0.070175,
        s: 0.345455,
        l: 0.215686,
        a: 1.0,
    },
    green: Hsla {
        h: 0.386207,
        s: 0.601660,
        l: 0.527451,
        a: 1.0,
    },
    green_soft: Hsla {
        h: 0.393333,
        s: 0.454545,
        l: 0.107843,
        a: 1.0,
    },
    green_border: Hsla {
        h: 0.425926,
        s: 0.391304,
        l: 0.180392,
        a: 1.0,
    },
    red: Hsla {
        h: 0.000000,
        s: 0.826087,
        l: 0.639216,
        a: 1.0,
    },
    yellow: Hsla {
        h: 0.126106,
        s: 0.933884,
        l: 0.474510,
        a: 1.0,
    },
    transparent_overlay: Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.0,
        a: 0.55,
    },
};

static IS_DARK: AtomicBool = AtomicBool::new(false);

pub fn set_dark(dark: bool) {
    IS_DARK.store(dark, Ordering::Relaxed);
}

pub fn is_dark() -> bool {
    IS_DARK.load(Ordering::Relaxed)
}

#[inline]
pub fn current() -> &'static Palette {
    if is_dark() { &DARK } else { &LIGHT }
}

// Token accessors. These shadow the old `pub const` API; replacing `palette::FOO`
// with `palette::foo()` at every call site lets the value vary with theme mode
// without threading a `cx` reference through every widget builder.
#[inline]
pub fn bg() -> Hsla {
    current().bg
}
#[inline]
pub fn panel() -> Hsla {
    current().panel
}
#[inline]
pub fn sidebar() -> Hsla {
    current().sidebar
}
#[inline]
pub fn border() -> Hsla {
    current().border
}
#[inline]
pub fn border_strong() -> Hsla {
    current().border_strong
}
#[inline]
pub fn text() -> Hsla {
    current().text
}
#[inline]
pub fn text_muted() -> Hsla {
    current().text_muted
}
#[inline]
pub fn text_faint() -> Hsla {
    current().text_faint
}
#[inline]
pub fn blue() -> Hsla {
    current().blue
}
#[inline]
pub fn blue_hover() -> Hsla {
    current().blue_hover
}
#[inline]
pub fn blue_soft() -> Hsla {
    current().blue_soft
}
#[inline]
pub fn blue_border() -> Hsla {
    current().blue_border
}
#[inline]
pub fn orange() -> Hsla {
    current().orange
}
#[inline]
pub fn orange_deep() -> Hsla {
    current().orange_deep
}
#[inline]
pub fn orange_soft() -> Hsla {
    current().orange_soft
}
#[inline]
pub fn orange_border() -> Hsla {
    current().orange_border
}
#[inline]
pub fn green() -> Hsla {
    current().green
}
#[inline]
pub fn green_soft() -> Hsla {
    current().green_soft
}
#[inline]
pub fn green_border() -> Hsla {
    current().green_border
}
#[inline]
pub fn red() -> Hsla {
    current().red
}
#[inline]
pub fn yellow() -> Hsla {
    current().yellow
}
#[inline]
pub fn transparent_overlay() -> Hsla {
    current().transparent_overlay
}
