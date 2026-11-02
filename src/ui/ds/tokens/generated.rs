//! Generated from `design/tokens.json` by `codegen.rs`. Do not edit: change the
//! JSON and run `FP_UPDATE_GOLDEN=1 cargo test golden`.

// The numbers are colors and sizes; one may land near a mathematical
// constant by chance.
#![allow(clippy::approx_constant)]

use gpui::Hsla;

use super::{Color, Shadow};

/// The standard text size in points.
pub const TEXT_SIZE_STANDARD: f32 = 13.0;

/// The large text size in points.
pub const TEXT_SIZE_LARGE: f32 = 15.0;

/// The primitive colors. UI code reads the tokens in [`ColorSet`] instead.
pub mod primitive {
    use super::{Color, Hsla};

    pub const BLUE_272: Color = Color {
        rgb: 0x1A2640,
        hsla: Hsla {
            h: 0.61403507,
            s: 0.42222226,
            l: 0.1764706,
            a: 1.0,
        },
    };

    pub const BLUE_418: Color = Color {
        rgb: 0x13479B,
        hsla: Hsla {
            h: 0.6029412,
            s: 0.78160924,
            l: 0.34117648,
            a: 1.0,
        },
    };

    pub const BLUE_508: Color = Color {
        rgb: 0x2E62B8,
        hsla: Hsla {
            h: 0.6038647,
            s: 0.6,
            l: 0.4509804,
            a: 1.0,
        },
    };

    pub const BLUE_731: Color = Color {
        rgb: 0x7FA8F0,
        hsla: Hsla {
            h: 0.6061947,
            s: 0.7902098,
            l: 0.71960783,
            a: 1.0,
        },
    };

    pub const BLUE_781: Color = Color {
        rgb: 0x90B8FF,
        hsla: Hsla {
            h: 0.6066066,
            s: 0.9999999,
            l: 0.7823529,
            a: 1.0,
        },
    };

    pub const BLUE_941: Color = Color {
        rgb: 0xE4ECF8,
        hsla: Hsla {
            h: 0.59999996,
            s: 0.5882353,
            l: 0.93333334,
            a: 1.0,
        },
    };

    pub const BRAND_653: Color = Color {
        rgb: 0xF74C00,
        hsla: Hsla {
            h: 0.051282052,
            s: 1.0,
            l: 0.48431373,
            a: 1.0,
        },
    };

    pub const CARMINE_197: Color = Color {
        rgb: 0x2A0710,
        hsla: Hsla {
            h: 0.95714283,
            s: 0.71428573,
            l: 0.09607843,
            a: 1.0,
        },
    };

    pub const CARMINE_269: Color = Color {
        rgb: 0x3E1820,
        hsla: Hsla {
            h: 0.96491224,
            s: 0.44186044,
            l: 0.16862746,
            a: 1.0,
        },
    };

    pub const CARMINE_435: Color = Color {
        rgb: 0x990033,
        hsla: Hsla {
            h: 0.9444444,
            s: 1.0,
            l: 0.3,
            a: 1.0,
        },
    };

    pub const CARMINE_486: Color = Color {
        rgb: 0xAD1940,
        hsla: Hsla {
            h: 0.9560811,
            s: 0.7474748,
            l: 0.3882353,
            a: 1.0,
        },
    };

    pub const CARMINE_506: Color = Color {
        rgb: 0xB42245,
        hsla: Hsla {
            h: 0.96004564,
            s: 0.682243,
            l: 0.41960785,
            a: 1.0,
        },
    };

    pub const CARMINE_724: Color = Color {
        rgb: 0xFF6F8E,
        hsla: Hsla {
            h: 0.9641204,
            s: 1.0,
            l: 0.7176471,
            a: 1.0,
        },
    };

    pub const CARMINE_727: Color = Color {
        rgb: 0xFF718F,
        hsla: Hsla {
            h: 0.96478873,
            s: 1.0,
            l: 0.72156864,
            a: 1.0,
        },
    };

    pub const CARMINE_795: Color = Color {
        rgb: 0xFF9AAB,
        hsla: Hsla {
            h: 0.9719472,
            s: 1.0000001,
            l: 0.8019608,
            a: 1.0,
        },
    };

    pub const CARMINE_938: Color = Color {
        rgb: 0xFAE4E9,
        hsla: Hsla {
            h: 0.96212125,
            s: 0.6875,
            l: 0.9372549,
            a: 1.0,
        },
    };

    pub const COPPER_178: Color = Color {
        rgb: 0x1C0D04,
        hsla: Hsla {
            h: 0.062499996,
            s: 0.75,
            l: 0.0627451,
            a: 1.0,
        },
    };

    pub const COPPER_285: Color = Color {
        rgb: 0x3A2418,
        hsla: Hsla {
            h: 0.05882354,
            s: 0.4146341,
            l: 0.16078432,
            a: 1.0,
        },
    };

    pub const COPPER_426: Color = Color {
        rgb: 0x862E01,
        hsla: Hsla {
            h: 0.056390975,
            s: 0.9851852,
            l: 0.2647059,
            a: 1.0,
        },
    };

    pub const COPPER_439: Color = Color {
        rgb: 0x8B3101,
        hsla: Hsla {
            h: 0.05797101,
            s: 0.9857143,
            l: 0.27450982,
            a: 1.0,
        },
    };

    pub const COPPER_479: Color = Color {
        rgb: 0x9D3801,
        hsla: Hsla {
            h: 0.058760684,
            s: 0.98734176,
            l: 0.30980393,
            a: 1.0,
        },
    };

    pub const COPPER_487: Color = Color {
        rgb: 0x9E3C0C,
        hsla: Hsla {
            h: 0.05479452,
            s: 0.85882354,
            l: 0.33333334,
            a: 1.0,
        },
    };

    pub const COPPER_509: Color = Color {
        rgb: 0xA8400D,
        hsla: Hsla {
            h: 0.054838713,
            s: 0.8563536,
            l: 0.35490197,
            a: 1.0,
        },
    };

    pub const COPPER_532: Color = Color {
        rgb: 0xB24501,
        hsla: Hsla {
            h: 0.06403013,
            s: 0.9888268,
            l: 0.3509804,
            a: 1.0,
        },
    };

    pub const COPPER_546: Color = Color {
        rgb: 0xB9470F,
        hsla: Hsla {
            h: 0.05490196,
            s: 0.85,
            l: 0.39215687,
            a: 1.0,
        },
    };

    pub const COPPER_553: Color = Color {
        rgb: 0xC2410C,
        hsla: Hsla {
            h: 0.0485348,
            s: 0.88349515,
            l: 0.40392157,
            a: 1.0,
        },
    };

    pub const COPPER_590: Color = Color {
        rgb: 0xC8551A,
        hsla: Hsla {
            h: 0.056513414,
            s: 0.7699115,
            l: 0.44313726,
            a: 1.0,
        },
    };

    pub const COPPER_697: Color = Color {
        rgb: 0xF0753A,
        hsla: Hsla {
            h: 0.0540293,
            s: 0.85849065,
            l: 0.58431375,
            a: 1.0,
        },
    };

    pub const COPPER_714: Color = Color {
        rgb: 0xF67A40,
        hsla: Hsla {
            h: 0.053113554,
            s: 0.91,
            l: 0.60784316,
            a: 1.0,
        },
    };

    pub const COPPER_739: Color = Color {
        rgb: 0xF58A55,
        hsla: Hsla {
            h: 0.05520834,
            s: 0.8888889,
            l: 0.64705884,
            a: 1.0,
        },
    };

    pub const COPPER_754: Color = Color {
        rgb: 0xF5925F,
        hsla: Hsla {
            h: 0.056666676,
            s: 0.88235295,
            l: 0.6666667,
            a: 1.0,
        },
    };

    pub const COPPER_763: Color = Color {
        rgb: 0xFF9060,
        hsla: Hsla {
            h: 0.050314467,
            s: 1.0,
            l: 0.6882353,
            a: 1.0,
        },
    };

    pub const COPPER_787: Color = Color {
        rgb: 0xF6A27A,
        hsla: Hsla {
            h: 0.05376345,
            s: 0.8732395,
            l: 0.72156864,
            a: 1.0,
        },
    };

    pub const COPPER_790: Color = Color {
        rgb: 0xFF9F6E,
        hsla: Hsla {
            h: 0.05632184,
            s: 1.0,
            l: 0.71568626,
            a: 1.0,
        },
    };

    pub const COPPER_936: Color = Color {
        rgb: 0xF8E6DA,
        hsla: Hsla {
            h: 0.06666667,
            s: 0.6818182,
            l: 0.9137255,
            a: 1.0,
        },
    };

    pub const MOSS_505: Color = Color {
        rgb: 0x487315,
        hsla: Hsla {
            h: 0.2429078,
            s: 0.6911765,
            l: 0.26666668,
            a: 1.0,
        },
    };

    pub const MOSS_528: Color = Color {
        rgb: 0x4E7A1E,
        hsla: Hsla {
            h: 0.2463768,
            s: 0.6052632,
            l: 0.29803923,
            a: 1.0,
        },
    };

    pub const MOSS_791: Color = Color {
        rgb: 0x9FCC68,
        hsla: Hsla {
            h: 0.24166667,
            s: 0.49504954,
            l: 0.6039216,
            a: 1.0,
        },
    };

    pub const OCHRE_303: Color = Color {
        rgb: 0x3A2C12,
        hsla: Hsla {
            h: 0.10833334,
            s: 0.52631575,
            l: 0.14901961,
            a: 1.0,
        },
    };

    pub const OCHRE_417: Color = Color {
        rgb: 0x6B4200,
        hsla: Hsla {
            h: 0.102803744,
            s: 1.0,
            l: 0.20980392,
            a: 1.0,
        },
    };

    pub const OCHRE_514: Color = Color {
        rgb: 0x8F5A00,
        hsla: Hsla {
            h: 0.10489511,
            s: 1.0,
            l: 0.28039217,
            a: 1.0,
        },
    };

    pub const OCHRE_793: Color = Color {
        rgb: 0xE7B24A,
        hsla: Hsla {
            h: 0.110403396,
            s: 0.7658537,
            l: 0.5980392,
            a: 1.0,
        },
    };

    pub const OCHRE_949: Color = Color {
        rgb: 0xF8EDD6,
        hsla: Hsla {
            h: 0.1127451,
            s: 0.7083333,
            l: 0.90588236,
            a: 1.0,
        },
    };

    pub const PATINA_285: Color = Color {
        rgb: 0x14302A,
        hsla: Hsla {
            h: 0.46428573,
            s: 0.41176468,
            l: 0.13333334,
            a: 1.0,
        },
    };

    pub const PATINA_401: Color = Color {
        rgb: 0x015545,
        hsla: Hsla {
            h: 0.46825397,
            s: 0.9767442,
            l: 0.16862746,
            a: 1.0,
        },
    };

    pub const PATINA_488: Color = Color {
        rgb: 0x1B6F5C,
        hsla: Hsla {
            h: 0.46230158,
            s: 0.6086956,
            l: 0.27058825,
            a: 1.0,
        },
    };

    pub const PATINA_576: Color = Color {
        rgb: 0x278C72,
        hsla: Hsla {
            h: 0.4570957,
            s: 0.5642458,
            l: 0.3509804,
            a: 1.0,
        },
    };

    pub const PATINA_782: Color = Color {
        rgb: 0x5FCFB4,
        hsla: Hsla {
            h: 0.45982143,
            s: 0.53846157,
            l: 0.5921569,
            a: 1.0,
        },
    };

    pub const PATINA_946: Color = Color {
        rgb: 0xE2F1EC,
        hsla: Hsla {
            h: 0.44444445,
            s: 0.34883708,
            l: 0.91568625,
            a: 1.0,
        },
    };

    pub const PLUM_509: Color = Color {
        rgb: 0x8C3FA0,
        hsla: Hsla {
            h: 0.7989691,
            s: 0.43497762,
            l: 0.4372549,
            a: 1.0,
        },
    };

    pub const PLUM_743: Color = Color {
        rgb: 0xD38DE2,
        hsla: Hsla {
            h: 0.8039215,
            s: 0.59440553,
            l: 0.71960783,
            a: 1.0,
        },
    };

    pub const SAND_178: Color = Color {
        rgb: 0x121110,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.058823522,
            l: 0.06666667,
            a: 1.0,
        },
    };

    pub const SAND_197: Color = Color {
        rgb: 0x171513,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.09523809,
            l: 0.08235294,
            a: 1.0,
        },
    };

    pub const SAND_201: Color = Color {
        rgb: 0x171614,
        hsla: Hsla {
            h: 0.11111111,
            s: 0.06976744,
            l: 0.08431373,
            a: 1.0,
        },
    };

    pub const SAND_221: Color = Color {
        rgb: 0x1E1A16,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.15384614,
            l: 0.101960786,
            a: 1.0,
        },
    };

    pub const SAND_224: Color = Color {
        rgb: 0x1D1B19,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.07407407,
            l: 0.105882354,
            a: 1.0,
        },
    };

    pub const SAND_249: Color = Color {
        rgb: 0x23211E,
        hsla: Hsla {
            h: 0.100000024,
            s: 0.076923095,
            l: 0.12745099,
            a: 1.0,
        },
    };

    pub const SAND_262: Color = Color {
        rgb: 0x262422,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.05555555,
            l: 0.14117648,
            a: 1.0,
        },
    };

    pub const SAND_272: Color = Color {
        rgb: 0x2A2622,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.105263144,
            l: 0.14901961,
            a: 1.0,
        },
    };

    pub const SAND_290: Color = Color {
        rgb: 0x2D2B29,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.046511624,
            l: 0.16862746,
            a: 1.0,
        },
    };

    pub const SAND_298: Color = Color {
        rgb: 0x2F2D2A,
        hsla: Hsla {
            h: 0.1,
            s: 0.05617977,
            l: 0.17450981,
            a: 1.0,
        },
    };

    pub const SAND_343: Color = Color {
        rgb: 0x3B3835,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.053571425,
            l: 0.21960784,
            a: 1.0,
        },
    };

    pub const SAND_414: Color = Color {
        rgb: 0x524A41,
        hsla: Hsla {
            h: 0.0882353,
            s: 0.11564624,
            l: 0.2882353,
            a: 1.0,
        },
    };

    pub const SAND_461: Color = Color {
        rgb: 0x5F574D,
        hsla: Hsla {
            h: 0.0925926,
            s: 0.10465115,
            l: 0.3372549,
            a: 1.0,
        },
    };

    pub const SAND_500: Color = Color {
        rgb: 0x6A6257,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.09844559,
            l: 0.37843138,
            a: 1.0,
        },
    };

    pub const SAND_514: Color = Color {
        rgb: 0x6E665B,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.09452736,
            l: 0.39411765,
            a: 1.0,
        },
    };

    pub const SAND_558: Color = Color {
        rgb: 0x777370,
        hsla: Hsla {
            h: 0.071428575,
            s: 0.030303027,
            l: 0.45294118,
            a: 1.0,
        },
    };

    pub const SAND_573: Color = Color {
        rgb: 0x7E776F,
        hsla: Hsla {
            h: 0.08888889,
            s: 0.06329113,
            l: 0.46470588,
            a: 1.0,
        },
    };

    pub const SAND_593: Color = Color {
        rgb: 0x857D71,
        hsla: Hsla {
            h: 0.099999964,
            s: 0.08130083,
            l: 0.48235297,
            a: 1.0,
        },
    };

    pub const SAND_613: Color = Color {
        rgb: 0x89837A,
        hsla: Hsla {
            h: 0.10000003,
            s: 0.059760984,
            l: 0.50784314,
            a: 1.0,
        },
    };

    pub const SAND_660: Color = Color {
        rgb: 0x989188,
        hsla: Hsla {
            h: 0.09375,
            s: 0.072072074,
            l: 0.5647059,
            a: 1.0,
        },
    };

    pub const SAND_664: Color = Color {
        rgb: 0x9A9287,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.08597285,
            l: 0.5666667,
            a: 1.0,
        },
    };

    pub const SAND_719: Color = Color {
        rgb: 0xABA398,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.10160427,
            l: 0.6333333,
            a: 1.0,
        },
    };

    pub const SAND_780: Color = Color {
        rgb: 0xBEB6AB,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.12751679,
            l: 0.7078432,
            a: 1.0,
        },
    };

    pub const SAND_866: Color = Color {
        rgb: 0xD9D2C7,
        hsla: Hsla {
            h: 0.10185185,
            s: 0.19148937,
            l: 0.8156863,
            a: 1.0,
        },
    };

    pub const SAND_893: Color = Color {
        rgb: 0xE9D9C6,
        hsla: Hsla {
            h: 0.09047619,
            s: 0.4430379,
            l: 0.845098,
            a: 1.0,
        },
    };

    pub const SAND_909: Color = Color {
        rgb: 0xE6E0D6,
        hsla: Hsla {
            h: 0.104166664,
            s: 0.24242425,
            l: 0.87058824,
            a: 1.0,
        },
    };

    pub const SAND_927: Color = Color {
        rgb: 0xEBE6DE,
        hsla: Hsla {
            h: 0.102564104,
            s: 0.24528308,
            l: 0.89607847,
            a: 1.0,
        },
    };

    pub const SAND_938: Color = Color {
        rgb: 0xEEEAE4,
        hsla: Hsla {
            h: 0.1,
            s: 0.22727273,
            l: 0.9137255,
            a: 1.0,
        },
    };

    pub const SAND_941: Color = Color {
        rgb: 0xEFEBE4,
        hsla: Hsla {
            h: 0.1060606,
            s: 0.25581387,
            l: 0.91568625,
            a: 1.0,
        },
    };

    pub const SAND_947: Color = Color {
        rgb: 0xF1EDE6,
        hsla: Hsla {
            h: 0.1060606,
            s: 0.28205118,
            l: 0.9235294,
            a: 1.0,
        },
    };

    pub const SAND_953: Color = Color {
        rgb: 0xF6EEE4,
        hsla: Hsla {
            h: 0.0925926,
            s: 0.5,
            l: 0.92941177,
            a: 1.0,
        },
    };

    pub const SAND_974: Color = Color {
        rgb: 0xF8F6F2,
        hsla: Hsla {
            h: 0.11111111,
            s: 0.3,
            l: 0.9607843,
            a: 1.0,
        },
    };

    pub const SAND_1000: Color = Color {
        rgb: 0xFFFFFF,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 1.0,
            a: 1.0,
        },
    };

    pub const SLATE_508: Color = Color {
        rgb: 0x596777,
        hsla: Hsla {
            h: 0.5888889,
            s: 0.14423075,
            l: 0.40784314,
            a: 1.0,
        },
    };

    pub const SLATE_755: Color = Color {
        rgb: 0xA4B1C0,
        hsla: Hsla {
            h: 0.58928573,
            s: 0.18181819,
            l: 0.69803923,
            a: 1.0,
        },
    };

    pub const TEAL_509: Color = Color {
        rgb: 0x0B7284,
        hsla: Hsla {
            h: 0.5247934,
            s: 0.84615386,
            l: 0.28039217,
            a: 1.0,
        },
    };

    pub const TEAL_512: Color = Color {
        rgb: 0x0E7385,
        hsla: Hsla {
            h: 0.5252101,
            s: 0.8095238,
            l: 0.2882353,
            a: 1.0,
        },
    };

    pub const TEAL_746: Color = Color {
        rgb: 0x4CBFCF,
        hsla: Hsla {
            h: 0.52035624,
            s: 0.5770925,
            l: 0.55490196,
            a: 1.0,
        },
    };
}

/// Points at text size 13, set in code as rems(value / 13) so they grow with the
/// text size. Radii, strokes and the focus ring are pixels and stay as they are.
pub mod dimension {
    pub const FOCUS_GAP: f32 = 1.0;
    pub const FOCUS_WIDTH: f32 = 2.0;
    pub const HEIGHT_CAPSULE: f32 = 44.0;
    pub const HEIGHT_CHIP: f32 = 18.0;
    pub const HEIGHT_CONTROL: f32 = 28.0;
    pub const HEIGHT_CONTROL_LG: f32 = 32.0;
    pub const HEIGHT_CONTROL_SM: f32 = 24.0;
    pub const HEIGHT_FIELD_ROW: f32 = 48.0;
    pub const HEIGHT_KEYCAP: f32 = 18.0;
    pub const HEIGHT_PANEL_MAX: f32 = 460.0;
    pub const HEIGHT_ROW: f32 = 44.0;
    pub const HEIGHT_ROW_COMPACT: f32 = 28.0;
    pub const HEIGHT_ROW_PANEL: f32 = 44.0;
    pub const HEIGHT_ROW_PANEL_COMPACT: f32 = 32.0;
    pub const HEIGHT_SIDEBAR_ITEM: f32 = 28.0;
    pub const HEIGHT_TILE: f32 = 28.0;
    pub const HEIGHT_TITLEZONE: f32 = 52.0;
    pub const HEIGHT_WINDOW_MIN: f32 = 600.0;
    pub const RADIUS_LG: f32 = 12.0;
    pub const RADIUS_MD: f32 = 8.0;
    pub const RADIUS_PILL: f32 = 22.0;
    pub const RADIUS_SM: f32 = 6.0;
    pub const RADIUS_XL: f32 = 16.0;
    pub const RADIUS_XS: f32 = 4.0;
    pub const SPACE_2: f32 = 2.0;
    pub const SPACE_4: f32 = 4.0;
    pub const SPACE_6: f32 = 6.0;
    pub const SPACE_8: f32 = 8.0;
    pub const SPACE_12: f32 = 12.0;
    pub const SPACE_16: f32 = 16.0;
    pub const SPACE_20: f32 = 20.0;
    pub const SPACE_24: f32 = 24.0;
    pub const SPACE_32: f32 = 32.0;
    pub const SPACE_40: f32 = 40.0;
    pub const STROKE_ICON: f32 = 1.5;
    pub const STROKE_OTP: f32 = 2.0;
    pub const STROKE_THREAD: f32 = 2.0;
    pub const WIDTH_DETAIL_MIN: f32 = 360.0;
    pub const WIDTH_DETAIL_WRAP: f32 = 340.0;
    pub const WIDTH_FIELD_LABEL_MIN: f32 = 108.0;
    pub const WIDTH_HUD_MAX: f32 = 420.0;
    pub const WIDTH_LIST: f32 = 320.0;
    pub const WIDTH_MATRIX_CELL: f32 = 120.0;
    pub const WIDTH_PANEL: f32 = 680.0;
    pub const WIDTH_SIDEBAR: f32 = 220.0;
    pub const WIDTH_WINDOW_MIN: f32 = 900.0;
}

/// Every color token for one mode, plus the shadows that go with it.
#[derive(Debug, PartialEq)]
pub struct ColorSet {
    // Semantic tokens.
    /// The one copper surface
    pub accent_fill: Color,
    /// Copper surface under the pointer
    pub accent_fill_hover: Color,
    /// Text on copper
    pub accent_on: Color,
    /// Ghost buttons, links
    pub accent_text: Color,
    /// Popover, menu, HUD, companion; always with a shadow
    pub bg_elevated: Color,
    /// Hover, secondary buttons, segment track, tile
    pub bg_hover: Color,
    /// Keycap
    pub bg_keycap: Color,
    /// Selection (soft copper)
    pub bg_selected: Color,
    /// Selection while the window is not focused
    pub bg_selected_unfocused: Color,
    /// Sidebar, without a border
    pub bg_sidebar: Color,
    /// List, detail, panel
    pub bg_surface: Color,
    /// Window and title zone
    pub bg_window: Color,
    /// Field and control borders
    pub border_control: Color,
    /// Keycap border
    pub border_keycap: Color,
    /// Separators; decorative, at least 3:1 with increased contrast
    pub border_separator: Color,
    /// Graphics and icons only, never text
    pub brand_mark: Color,
    /// Focus ring
    pub focus_ring: Color,
    /// Ochre surface
    pub status_attention_soft: Color,
    /// Ochre: expiring, offline, production system
    pub status_attention_text: Color,
    /// Danger button
    pub status_danger_fill: Color,
    /// Text on the danger button
    pub status_danger_on: Color,
    /// Carmine surface
    pub status_danger_soft: Color,
    /// Carmine: errors, anything that cannot be undone
    pub status_danger_text: Color,
    /// Blue surface
    pub status_decision_soft: Color,
    /// Blue: a decision or a question
    pub status_decision_text: Color,
    /// Patina surface
    pub status_ok_soft: Color,
    /// Patina: up to date, saved, cleared
    pub status_ok_text: Color,
    /// Vault colour
    pub team_moss: Color,
    /// Vault colour
    pub team_plum: Color,
    /// Vault colour
    pub team_slate: Color,
    /// Vault colour
    pub team_teal: Color,
    /// Titles, values
    pub text_primary: Color,
    /// Second line, icons, captions
    pub text_secondary: Color,
    /// Labels, counters, placeholders; same as text.secondary with increased contrast
    pub text_tertiary: Color,
    // Aliases: a fixed name for a use, pointing at a semantic token.
    /// Thread while a secret is on the clipboard
    pub clipboard_thread: Color,
    /// End of the thread once the clipboard is cleared
    pub clipboard_thread_end: Color,
    /// One-time code ring
    pub otp_ring: Color,
    /// One-time code ring shortly before the code expires
    pub otp_ring_warn: Color,
    /// Track of the one-time code ring
    pub otp_track: Color,
    /// Production system chip, surface
    pub prd_chip_bg: Color,
    /// Production system chip, text
    pub prd_chip_text: Color,
    /// Digits in a revealed secret
    pub secret_digit: Color,
    /// Symbols in a revealed secret
    pub secret_symbol: Color,
    /// Dashed ring of any other vault
    pub vault_other: Color,
    /// Ring of the personal vault
    pub vault_personal: Color,
    // Brand graphics only, never text and never a UI surface.
    /// Ferris, cheeks
    pub brand_ferris_cheek: Color,
    /// Treasure chest on the lock screen
    pub brand_ferris_chest: Color,
    /// Ferris, claws
    pub brand_ferris_claw: Color,
    /// Ferris, eyes and smile
    pub brand_ferris_eye: Color,
    /// Ferris, glint in the eyes
    pub brand_ferris_glint: Color,
    /// Ferris, key
    pub brand_ferris_key: Color,
    /// Bands, fittings and lock plate of the chest
    pub brand_ferris_metal: Color,
    /// Ferris, shell
    pub brand_ferris_shell: Color,
    /// App icon, bottom of the background gradient
    pub brand_icon_bg_bottom: Color,
    /// App icon, top of the background gradient
    pub brand_icon_bg_top: Color,
    /// App icon, cheeks
    pub brand_icon_cheek: Color,
    /// App icon, claws
    pub brand_icon_claw: Color,
    /// App icon, eyes
    pub brand_icon_eye: Color,
    /// App icon, glint
    pub brand_icon_glint: Color,
    /// App icon, key
    pub brand_icon_key: Color,
    /// App icon, shell
    pub brand_icon_shell: Color,
    // Component tokens, where a component differs from the semantic layer.
    pub banner_attention_bg: Color,
    pub banner_attention_glyph: Color,
    pub banner_attention_text: Color,
    pub banner_danger_bg: Color,
    pub banner_danger_glyph: Color,
    pub banner_danger_text: Color,
    pub banner_decision_bg: Color,
    pub banner_decision_glyph: Color,
    pub banner_decision_text: Color,
    pub banner_ok_bg: Color,
    pub banner_ok_glyph: Color,
    pub banner_ok_text: Color,
    pub button_danger_bg: Color,
    /// The increased-contrast step of the fill: 7.02:1 light, 7.05:1 dark
    pub button_danger_hover: Color,
    pub button_danger_text: Color,
    /// A disabled button loses its colour
    pub button_disabled_bg: Color,
    pub button_disabled_text: Color,
    pub button_ghost_hover_bg: Color,
    /// A verb inside a row
    pub button_ghost_text: Color,
    /// The one copper surface
    pub button_primary_bg: Color,
    pub button_primary_hover: Color,
    pub button_primary_text: Color,
    pub button_quiet_hover_bg: Color,
    pub button_quiet_text: Color,
    pub button_secondary_bg: Color,
    pub button_secondary_hover: Color,
    pub button_secondary_text: Color,
    /// QAS, DEV, 2FA, detected
    pub chip_neutral_bg: Color,
    pub chip_neutral_text: Color,
    pub field_bg: Color,
    pub field_border: Color,
    pub field_border_error: Color,
    pub keycap_bg: Color,
    pub keycap_border: Color,
    /// Keycap inside the copper surface
    pub keycap_on_accent_border: Color,
    pub keycap_on_accent_text: Color,
    pub keycap_text: Color,
    /// GPUI Kit drop target
    pub kit_drop_target: Color,
    /// GPUI Kit scrollbar thumb
    pub kit_scrollbar_thumb: Color,
    /// GPUI Kit scrollbar thumb under the pointer
    pub kit_scrollbar_thumb_hover: Color,
    /// GPUI Kit scrollbar track
    pub kit_scrollbar_track: Color,
    /// Text on GPUI Kit status surfaces, 5.78:1 to 9.99:1
    pub kit_status_on: Color,
    /// "(missing)" in the matrix
    pub matrix_missing_text: Color,
    pub row_selected_bg: Color,
    /// Inner border of a selected row, only with increased contrast
    pub row_selected_border: Color,
    pub row_selected_unfocused_bg: Color,
    /// Behind a modal dialog
    pub scrim: Color,
    pub segment_selected_bg: Color,
    pub segment_selected_ring: Color,
    pub segment_track: Color,
    pub switch_off_ring: Color,
    pub switch_off_track: Color,
    /// A switch that is on is never copper
    pub switch_on_bg: Color,
    pub switch_thumb: Color,
    /// Type tile
    pub tile_bg: Color,
    pub tile_glyph: Color,
    pub tile_selected_bg: Color,
    pub tile_selected_glyph: Color,
    // Shadows, one entry per layer.
    /// Panel, HUD, companion; drawn inside the transparent window, which is 48 wider on
    /// each side and 66 taller at the bottom
    pub floating: [Shadow; 2],
    /// Popover, menu
    pub raised: [Shadow; 2],
}

pub static LIGHT: ColorSet = ColorSet {
    accent_fill: primitive::COPPER_546,
    accent_fill_hover: primitive::COPPER_487,
    accent_on: primitive::SAND_1000,
    accent_text: primitive::COPPER_509,
    bg_elevated: primitive::SAND_1000,
    bg_hover: primitive::SAND_941,
    bg_keycap: primitive::SAND_947,
    bg_selected: primitive::COPPER_936,
    bg_selected_unfocused: primitive::SAND_927,
    bg_sidebar: primitive::SAND_947,
    bg_surface: primitive::SAND_1000,
    bg_window: primitive::SAND_974,
    border_control: primitive::SAND_593,
    border_keycap: primitive::SAND_866,
    border_separator: primitive::SAND_909,
    brand_mark: primitive::BRAND_653,
    focus_ring: primitive::COPPER_590,
    status_attention_soft: primitive::OCHRE_949,
    status_attention_text: primitive::OCHRE_514,
    status_danger_fill: primitive::CARMINE_506,
    status_danger_on: primitive::SAND_1000,
    status_danger_soft: primitive::CARMINE_938,
    status_danger_text: primitive::CARMINE_506,
    status_decision_soft: primitive::BLUE_941,
    status_decision_text: primitive::BLUE_508,
    status_ok_soft: primitive::PATINA_946,
    status_ok_text: primitive::PATINA_488,
    team_moss: primitive::MOSS_528,
    team_plum: primitive::PLUM_509,
    team_slate: primitive::SLATE_508,
    team_teal: primitive::TEAL_512,
    text_primary: primitive::SAND_221,
    text_secondary: primitive::SAND_461,
    text_tertiary: primitive::SAND_500,
    clipboard_thread: primitive::COPPER_590,
    clipboard_thread_end: primitive::PATINA_488,
    otp_ring: primitive::SAND_461,
    otp_ring_warn: primitive::OCHRE_514,
    otp_track: primitive::SAND_909,
    prd_chip_bg: primitive::OCHRE_949,
    prd_chip_text: primitive::OCHRE_514,
    secret_digit: primitive::BLUE_508,
    secret_symbol: primitive::COPPER_509,
    vault_other: primitive::SAND_461,
    vault_personal: primitive::SAND_461,
    brand_ferris_cheek: primitive::COPPER_787,
    brand_ferris_chest: primitive::PATINA_576,
    brand_ferris_claw: primitive::COPPER_553,
    brand_ferris_eye: primitive::SAND_272,
    brand_ferris_glint: primitive::SAND_1000,
    brand_ferris_key: primitive::PATINA_576,
    brand_ferris_metal: primitive::SAND_893,
    brand_ferris_shell: primitive::BRAND_653,
    brand_icon_bg_bottom: primitive::SAND_893,
    brand_icon_bg_top: primitive::SAND_953,
    brand_icon_cheek: primitive::COPPER_787,
    brand_icon_claw: primitive::COPPER_553,
    brand_icon_eye: primitive::SAND_272,
    brand_icon_glint: primitive::SAND_1000,
    brand_icon_key: primitive::PATINA_576,
    brand_icon_shell: primitive::BRAND_653,
    banner_attention_bg: primitive::OCHRE_949,
    banner_attention_glyph: primitive::OCHRE_514,
    banner_attention_text: primitive::SAND_221,
    banner_danger_bg: primitive::CARMINE_938,
    banner_danger_glyph: primitive::CARMINE_506,
    banner_danger_text: primitive::SAND_221,
    banner_decision_bg: primitive::BLUE_941,
    banner_decision_glyph: primitive::BLUE_508,
    banner_decision_text: primitive::SAND_221,
    banner_ok_bg: primitive::PATINA_946,
    banner_ok_glyph: primitive::PATINA_488,
    banner_ok_text: primitive::SAND_221,
    button_danger_bg: primitive::CARMINE_506,
    button_danger_hover: primitive::CARMINE_486,
    button_danger_text: primitive::SAND_1000,
    button_disabled_bg: primitive::SAND_941,
    button_disabled_text: primitive::SAND_500,
    button_ghost_hover_bg: primitive::SAND_941,
    button_ghost_text: primitive::COPPER_509,
    button_primary_bg: primitive::COPPER_546,
    button_primary_hover: primitive::COPPER_487,
    button_primary_text: primitive::SAND_1000,
    button_quiet_hover_bg: primitive::SAND_941,
    button_quiet_text: primitive::SAND_461,
    button_secondary_bg: primitive::SAND_941,
    button_secondary_hover: primitive::SAND_927,
    button_secondary_text: primitive::SAND_221,
    chip_neutral_bg: primitive::SAND_941,
    chip_neutral_text: primitive::SAND_461,
    field_bg: primitive::SAND_1000,
    field_border: primitive::SAND_593,
    field_border_error: primitive::CARMINE_506,
    keycap_bg: primitive::SAND_947,
    keycap_border: primitive::SAND_866,
    keycap_on_accent_border: Color {
        rgb: 0xFFFFFF,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 1.0,
            a: 0.4509804,
        },
    },
    keycap_on_accent_text: primitive::SAND_1000,
    keycap_text: primitive::SAND_461,
    kit_drop_target: Color {
        rgb: 0xB9470F,
        hsla: Hsla {
            h: 0.05490196,
            s: 0.85,
            l: 0.39215687,
            a: 0.12156863,
        },
    },
    kit_scrollbar_thumb: Color {
        rgb: 0x6A6257,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.09844559,
            l: 0.37843138,
            a: 0.5019608,
        },
    },
    kit_scrollbar_thumb_hover: Color {
        rgb: 0x5F574D,
        hsla: Hsla {
            h: 0.0925926,
            s: 0.10465115,
            l: 0.3372549,
            a: 0.8,
        },
    },
    kit_scrollbar_track: Color {
        rgb: 0x000000,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        },
    },
    kit_status_on: primitive::SAND_1000,
    matrix_missing_text: primitive::SAND_500,
    row_selected_bg: primitive::COPPER_936,
    row_selected_border: Color {
        rgb: 0x000000,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        },
    },
    row_selected_unfocused_bg: primitive::SAND_927,
    scrim: Color {
        rgb: 0x1E1A16,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.15384614,
            l: 0.101960786,
            a: 0.3019608,
        },
    },
    segment_selected_bg: primitive::SAND_1000,
    segment_selected_ring: primitive::SAND_866,
    segment_track: primitive::SAND_941,
    switch_off_ring: primitive::SAND_593,
    switch_off_track: primitive::SAND_927,
    switch_on_bg: primitive::SAND_221,
    switch_thumb: primitive::SAND_1000,
    tile_bg: primitive::SAND_941,
    tile_glyph: primitive::SAND_461,
    tile_selected_bg: primitive::SAND_1000,
    tile_selected_glyph: primitive::SAND_221,
    floating: [
        Shadow {
            offset_y: 18.0,
            blur: 48.0,
            spread: 0.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.25882354,
                },
            },
            inset: false,
        },
        Shadow {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.101960786,
                },
            },
            inset: false,
        },
    ],
    raised: [
        Shadow {
            offset_y: 4.0,
            blur: 16.0,
            spread: 0.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.12156863,
                },
            },
            inset: false,
        },
        Shadow {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: primitive::SAND_909,
            inset: false,
        },
    ],
};

pub static DARK: ColorSet = ColorSet {
    accent_fill: primitive::COPPER_697,
    accent_fill_hover: primitive::COPPER_739,
    accent_on: primitive::COPPER_178,
    accent_text: primitive::COPPER_754,
    bg_elevated: primitive::SAND_262,
    bg_hover: primitive::SAND_249,
    bg_keycap: primitive::SAND_290,
    bg_selected: primitive::COPPER_285,
    bg_selected_unfocused: primitive::SAND_290,
    bg_sidebar: primitive::SAND_178,
    bg_surface: primitive::SAND_224,
    bg_window: primitive::SAND_201,
    border_control: primitive::SAND_573,
    border_keycap: primitive::SAND_343,
    border_separator: primitive::SAND_298,
    brand_mark: primitive::BRAND_653,
    focus_ring: primitive::COPPER_697,
    status_attention_soft: primitive::OCHRE_303,
    status_attention_text: primitive::OCHRE_793,
    status_danger_fill: primitive::CARMINE_724,
    status_danger_on: primitive::CARMINE_197,
    status_danger_soft: primitive::CARMINE_269,
    status_danger_text: primitive::CARMINE_724,
    status_decision_soft: primitive::BLUE_272,
    status_decision_text: primitive::BLUE_731,
    status_ok_soft: primitive::PATINA_285,
    status_ok_text: primitive::PATINA_782,
    team_moss: primitive::MOSS_791,
    team_plum: primitive::PLUM_743,
    team_slate: primitive::SLATE_755,
    team_teal: primitive::TEAL_746,
    text_primary: primitive::SAND_938,
    text_secondary: primitive::SAND_719,
    text_tertiary: primitive::SAND_664,
    clipboard_thread: primitive::COPPER_697,
    clipboard_thread_end: primitive::PATINA_782,
    otp_ring: primitive::SAND_719,
    otp_ring_warn: primitive::OCHRE_793,
    otp_track: primitive::SAND_298,
    prd_chip_bg: primitive::OCHRE_303,
    prd_chip_text: primitive::OCHRE_793,
    secret_digit: primitive::BLUE_731,
    secret_symbol: primitive::COPPER_754,
    vault_other: primitive::SAND_719,
    vault_personal: primitive::SAND_719,
    brand_ferris_cheek: primitive::COPPER_787,
    brand_ferris_chest: primitive::PATINA_576,
    brand_ferris_claw: primitive::COPPER_553,
    brand_ferris_eye: primitive::SAND_197,
    brand_ferris_glint: primitive::SAND_1000,
    brand_ferris_key: primitive::PATINA_576,
    brand_ferris_metal: primitive::SAND_893,
    brand_ferris_shell: primitive::BRAND_653,
    brand_icon_bg_bottom: primitive::SAND_197,
    brand_icon_bg_top: primitive::SAND_272,
    brand_icon_cheek: primitive::COPPER_787,
    brand_icon_claw: primitive::COPPER_553,
    brand_icon_eye: primitive::SAND_272,
    brand_icon_glint: primitive::SAND_1000,
    brand_icon_key: primitive::PATINA_576,
    brand_icon_shell: primitive::BRAND_653,
    banner_attention_bg: primitive::OCHRE_303,
    banner_attention_glyph: primitive::OCHRE_793,
    banner_attention_text: primitive::SAND_938,
    banner_danger_bg: primitive::CARMINE_269,
    banner_danger_glyph: primitive::CARMINE_724,
    banner_danger_text: primitive::SAND_938,
    banner_decision_bg: primitive::BLUE_272,
    banner_decision_glyph: primitive::BLUE_731,
    banner_decision_text: primitive::SAND_938,
    banner_ok_bg: primitive::PATINA_285,
    banner_ok_glyph: primitive::PATINA_782,
    banner_ok_text: primitive::SAND_938,
    button_danger_bg: primitive::CARMINE_724,
    button_danger_hover: primitive::CARMINE_727,
    button_danger_text: primitive::CARMINE_197,
    button_disabled_bg: primitive::SAND_249,
    button_disabled_text: primitive::SAND_664,
    button_ghost_hover_bg: primitive::SAND_249,
    button_ghost_text: primitive::COPPER_754,
    button_primary_bg: primitive::COPPER_697,
    button_primary_hover: primitive::COPPER_739,
    button_primary_text: primitive::COPPER_178,
    button_quiet_hover_bg: primitive::SAND_249,
    button_quiet_text: primitive::SAND_719,
    button_secondary_bg: primitive::SAND_249,
    button_secondary_hover: primitive::SAND_290,
    button_secondary_text: primitive::SAND_938,
    chip_neutral_bg: primitive::SAND_249,
    chip_neutral_text: primitive::SAND_719,
    field_bg: primitive::SAND_224,
    field_border: primitive::SAND_573,
    field_border_error: primitive::CARMINE_724,
    keycap_bg: primitive::SAND_290,
    keycap_border: primitive::SAND_343,
    keycap_on_accent_border: Color {
        rgb: 0x1C0D04,
        hsla: Hsla {
            h: 0.062499996,
            s: 0.75,
            l: 0.0627451,
            a: 0.4509804,
        },
    },
    keycap_on_accent_text: primitive::COPPER_178,
    keycap_text: primitive::SAND_719,
    kit_drop_target: Color {
        rgb: 0xF0753A,
        hsla: Hsla {
            h: 0.0540293,
            s: 0.85849065,
            l: 0.58431375,
            a: 0.12156863,
        },
    },
    kit_scrollbar_thumb: Color {
        rgb: 0x9A9287,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.08597285,
            l: 0.5666667,
            a: 0.5019608,
        },
    },
    kit_scrollbar_thumb_hover: Color {
        rgb: 0xABA398,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.10160427,
            l: 0.6333333,
            a: 0.8,
        },
    },
    kit_scrollbar_track: Color {
        rgb: 0x000000,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        },
    },
    kit_status_on: primitive::COPPER_178,
    matrix_missing_text: primitive::SAND_664,
    row_selected_bg: primitive::COPPER_285,
    row_selected_border: Color {
        rgb: 0x000000,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        },
    },
    row_selected_unfocused_bg: primitive::SAND_290,
    scrim: Color {
        rgb: 0x000000,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.54901963,
        },
    },
    segment_selected_bg: primitive::SAND_262,
    segment_selected_ring: primitive::SAND_343,
    segment_track: primitive::SAND_249,
    switch_off_ring: primitive::SAND_573,
    switch_off_track: primitive::SAND_290,
    switch_on_bg: primitive::SAND_938,
    switch_thumb: primitive::SAND_224,
    tile_bg: primitive::SAND_249,
    tile_glyph: primitive::SAND_719,
    tile_selected_bg: primitive::SAND_224,
    tile_selected_glyph: primitive::SAND_938,
    floating: [
        Shadow {
            offset_y: 18.0,
            blur: 48.0,
            spread: 0.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.6,
                },
            },
            inset: false,
        },
        Shadow {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: Color {
                rgb: 0xFFFFFF,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 1.0,
                    a: 0.101960786,
                },
            },
            inset: false,
        },
    ],
    raised: [
        Shadow {
            offset_y: 4.0,
            blur: 16.0,
            spread: 0.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.4,
                },
            },
            inset: false,
        },
        Shadow {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: Color {
                rgb: 0xFFFFFF,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 1.0,
                    a: 0.078431375,
                },
            },
            inset: true,
        },
    ],
};

pub static LIGHT_HC: ColorSet = ColorSet {
    accent_fill: primitive::COPPER_479,
    accent_fill_hover: primitive::COPPER_439,
    accent_on: primitive::SAND_1000,
    accent_text: primitive::COPPER_426,
    bg_elevated: primitive::SAND_1000,
    bg_hover: primitive::SAND_941,
    bg_keycap: primitive::SAND_947,
    bg_selected: primitive::COPPER_936,
    bg_selected_unfocused: primitive::SAND_927,
    bg_sidebar: primitive::SAND_947,
    bg_surface: primitive::SAND_1000,
    bg_window: primitive::SAND_974,
    border_control: primitive::SAND_514,
    border_keycap: primitive::SAND_613,
    border_separator: primitive::SAND_613,
    brand_mark: primitive::BRAND_653,
    focus_ring: primitive::COPPER_532,
    status_attention_soft: primitive::OCHRE_949,
    status_attention_text: primitive::OCHRE_417,
    status_danger_fill: primitive::CARMINE_486,
    status_danger_on: primitive::SAND_1000,
    status_danger_soft: primitive::CARMINE_938,
    status_danger_text: primitive::CARMINE_435,
    status_decision_soft: primitive::BLUE_941,
    status_decision_text: primitive::BLUE_418,
    status_ok_soft: primitive::PATINA_946,
    status_ok_text: primitive::PATINA_401,
    team_moss: primitive::MOSS_505,
    team_plum: primitive::PLUM_509,
    team_slate: primitive::SLATE_508,
    team_teal: primitive::TEAL_509,
    text_primary: primitive::SAND_221,
    text_secondary: primitive::SAND_414,
    text_tertiary: primitive::SAND_414,
    clipboard_thread: primitive::COPPER_532,
    clipboard_thread_end: primitive::PATINA_401,
    otp_ring: primitive::SAND_414,
    otp_ring_warn: primitive::OCHRE_417,
    otp_track: primitive::SAND_613,
    prd_chip_bg: primitive::OCHRE_949,
    prd_chip_text: primitive::OCHRE_417,
    secret_digit: primitive::BLUE_418,
    secret_symbol: primitive::COPPER_426,
    vault_other: primitive::SAND_414,
    vault_personal: primitive::SAND_414,
    brand_ferris_cheek: primitive::COPPER_787,
    brand_ferris_chest: primitive::PATINA_576,
    brand_ferris_claw: primitive::COPPER_553,
    brand_ferris_eye: primitive::SAND_272,
    brand_ferris_glint: primitive::SAND_1000,
    brand_ferris_key: primitive::PATINA_576,
    brand_ferris_metal: primitive::SAND_893,
    brand_ferris_shell: primitive::BRAND_653,
    brand_icon_bg_bottom: primitive::SAND_893,
    brand_icon_bg_top: primitive::SAND_953,
    brand_icon_cheek: primitive::COPPER_787,
    brand_icon_claw: primitive::COPPER_553,
    brand_icon_eye: primitive::SAND_272,
    brand_icon_glint: primitive::SAND_1000,
    brand_icon_key: primitive::PATINA_576,
    brand_icon_shell: primitive::BRAND_653,
    banner_attention_bg: primitive::OCHRE_949,
    banner_attention_glyph: primitive::OCHRE_417,
    banner_attention_text: primitive::SAND_221,
    banner_danger_bg: primitive::CARMINE_938,
    banner_danger_glyph: primitive::CARMINE_435,
    banner_danger_text: primitive::SAND_221,
    banner_decision_bg: primitive::BLUE_941,
    banner_decision_glyph: primitive::BLUE_418,
    banner_decision_text: primitive::SAND_221,
    banner_ok_bg: primitive::PATINA_946,
    banner_ok_glyph: primitive::PATINA_401,
    banner_ok_text: primitive::SAND_221,
    button_danger_bg: primitive::CARMINE_486,
    button_danger_hover: primitive::CARMINE_486,
    button_danger_text: primitive::SAND_1000,
    button_disabled_bg: primitive::SAND_941,
    button_disabled_text: primitive::SAND_414,
    button_ghost_hover_bg: primitive::SAND_941,
    button_ghost_text: primitive::COPPER_426,
    button_primary_bg: primitive::COPPER_479,
    button_primary_hover: primitive::COPPER_439,
    button_primary_text: primitive::SAND_1000,
    button_quiet_hover_bg: primitive::SAND_941,
    button_quiet_text: primitive::SAND_414,
    button_secondary_bg: primitive::SAND_941,
    button_secondary_hover: primitive::SAND_927,
    button_secondary_text: primitive::SAND_221,
    chip_neutral_bg: primitive::SAND_941,
    chip_neutral_text: primitive::SAND_414,
    field_bg: primitive::SAND_1000,
    field_border: primitive::SAND_514,
    field_border_error: primitive::CARMINE_435,
    keycap_bg: primitive::SAND_947,
    keycap_border: primitive::SAND_613,
    keycap_on_accent_border: Color {
        rgb: 0xFFFFFF,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 1.0,
            a: 0.4509804,
        },
    },
    keycap_on_accent_text: primitive::SAND_1000,
    keycap_text: primitive::SAND_414,
    kit_drop_target: Color {
        rgb: 0x9D3801,
        hsla: Hsla {
            h: 0.058760684,
            s: 0.98734176,
            l: 0.30980393,
            a: 0.12156863,
        },
    },
    kit_scrollbar_thumb: Color {
        rgb: 0x524A41,
        hsla: Hsla {
            h: 0.0882353,
            s: 0.11564624,
            l: 0.2882353,
            a: 0.5019608,
        },
    },
    kit_scrollbar_thumb_hover: Color {
        rgb: 0x524A41,
        hsla: Hsla {
            h: 0.0882353,
            s: 0.11564624,
            l: 0.2882353,
            a: 0.8,
        },
    },
    kit_scrollbar_track: Color {
        rgb: 0x000000,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        },
    },
    kit_status_on: primitive::SAND_1000,
    matrix_missing_text: primitive::SAND_414,
    row_selected_bg: primitive::COPPER_936,
    row_selected_border: primitive::SAND_514,
    row_selected_unfocused_bg: primitive::SAND_927,
    scrim: Color {
        rgb: 0x1E1A16,
        hsla: Hsla {
            h: 0.083333336,
            s: 0.15384614,
            l: 0.101960786,
            a: 0.3019608,
        },
    },
    segment_selected_bg: primitive::SAND_1000,
    segment_selected_ring: primitive::SAND_514,
    segment_track: primitive::SAND_941,
    switch_off_ring: primitive::SAND_514,
    switch_off_track: primitive::SAND_927,
    switch_on_bg: primitive::SAND_221,
    switch_thumb: primitive::SAND_1000,
    tile_bg: primitive::SAND_941,
    tile_glyph: primitive::SAND_414,
    tile_selected_bg: primitive::SAND_1000,
    tile_selected_glyph: primitive::SAND_221,
    floating: [
        Shadow {
            offset_y: 18.0,
            blur: 48.0,
            spread: 0.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.25882354,
                },
            },
            inset: false,
        },
        Shadow {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.101960786,
                },
            },
            inset: false,
        },
    ],
    raised: [
        Shadow {
            offset_y: 4.0,
            blur: 16.0,
            spread: 0.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.12156863,
                },
            },
            inset: false,
        },
        Shadow {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: primitive::SAND_613,
            inset: false,
        },
    ],
};

pub static DARK_HC: ColorSet = ColorSet {
    accent_fill: primitive::COPPER_714,
    accent_fill_hover: primitive::COPPER_763,
    accent_on: primitive::COPPER_178,
    accent_text: primitive::COPPER_790,
    bg_elevated: primitive::SAND_262,
    bg_hover: primitive::SAND_249,
    bg_keycap: primitive::SAND_290,
    bg_selected: primitive::COPPER_285,
    bg_selected_unfocused: primitive::SAND_290,
    bg_sidebar: primitive::SAND_178,
    bg_surface: primitive::SAND_224,
    bg_window: primitive::SAND_201,
    border_control: primitive::SAND_660,
    border_keycap: primitive::SAND_558,
    border_separator: primitive::SAND_558,
    brand_mark: primitive::BRAND_653,
    focus_ring: primitive::COPPER_697,
    status_attention_soft: primitive::OCHRE_303,
    status_attention_text: primitive::OCHRE_793,
    status_danger_fill: primitive::CARMINE_727,
    status_danger_on: primitive::CARMINE_197,
    status_danger_soft: primitive::CARMINE_269,
    status_danger_text: primitive::CARMINE_795,
    status_decision_soft: primitive::BLUE_272,
    status_decision_text: primitive::BLUE_781,
    status_ok_soft: primitive::PATINA_285,
    status_ok_text: primitive::PATINA_782,
    team_moss: primitive::MOSS_791,
    team_plum: primitive::PLUM_743,
    team_slate: primitive::SLATE_755,
    team_teal: primitive::TEAL_746,
    text_primary: primitive::SAND_938,
    text_secondary: primitive::SAND_780,
    text_tertiary: primitive::SAND_780,
    clipboard_thread: primitive::COPPER_697,
    clipboard_thread_end: primitive::PATINA_782,
    otp_ring: primitive::SAND_780,
    otp_ring_warn: primitive::OCHRE_793,
    otp_track: primitive::SAND_558,
    prd_chip_bg: primitive::OCHRE_303,
    prd_chip_text: primitive::OCHRE_793,
    secret_digit: primitive::BLUE_781,
    secret_symbol: primitive::COPPER_790,
    vault_other: primitive::SAND_780,
    vault_personal: primitive::SAND_780,
    brand_ferris_cheek: primitive::COPPER_787,
    brand_ferris_chest: primitive::PATINA_576,
    brand_ferris_claw: primitive::COPPER_553,
    brand_ferris_eye: primitive::SAND_197,
    brand_ferris_glint: primitive::SAND_1000,
    brand_ferris_key: primitive::PATINA_576,
    brand_ferris_metal: primitive::SAND_893,
    brand_ferris_shell: primitive::BRAND_653,
    brand_icon_bg_bottom: primitive::SAND_197,
    brand_icon_bg_top: primitive::SAND_272,
    brand_icon_cheek: primitive::COPPER_787,
    brand_icon_claw: primitive::COPPER_553,
    brand_icon_eye: primitive::SAND_272,
    brand_icon_glint: primitive::SAND_1000,
    brand_icon_key: primitive::PATINA_576,
    brand_icon_shell: primitive::BRAND_653,
    banner_attention_bg: primitive::OCHRE_303,
    banner_attention_glyph: primitive::OCHRE_793,
    banner_attention_text: primitive::SAND_938,
    banner_danger_bg: primitive::CARMINE_269,
    banner_danger_glyph: primitive::CARMINE_795,
    banner_danger_text: primitive::SAND_938,
    banner_decision_bg: primitive::BLUE_272,
    banner_decision_glyph: primitive::BLUE_781,
    banner_decision_text: primitive::SAND_938,
    banner_ok_bg: primitive::PATINA_285,
    banner_ok_glyph: primitive::PATINA_782,
    banner_ok_text: primitive::SAND_938,
    button_danger_bg: primitive::CARMINE_727,
    button_danger_hover: primitive::CARMINE_727,
    button_danger_text: primitive::CARMINE_197,
    button_disabled_bg: primitive::SAND_249,
    button_disabled_text: primitive::SAND_780,
    button_ghost_hover_bg: primitive::SAND_249,
    button_ghost_text: primitive::COPPER_790,
    button_primary_bg: primitive::COPPER_714,
    button_primary_hover: primitive::COPPER_763,
    button_primary_text: primitive::COPPER_178,
    button_quiet_hover_bg: primitive::SAND_249,
    button_quiet_text: primitive::SAND_780,
    button_secondary_bg: primitive::SAND_249,
    button_secondary_hover: primitive::SAND_290,
    button_secondary_text: primitive::SAND_938,
    chip_neutral_bg: primitive::SAND_249,
    chip_neutral_text: primitive::SAND_780,
    field_bg: primitive::SAND_224,
    field_border: primitive::SAND_660,
    field_border_error: primitive::CARMINE_795,
    keycap_bg: primitive::SAND_290,
    keycap_border: primitive::SAND_558,
    keycap_on_accent_border: Color {
        rgb: 0x1C0D04,
        hsla: Hsla {
            h: 0.062499996,
            s: 0.75,
            l: 0.0627451,
            a: 0.4509804,
        },
    },
    keycap_on_accent_text: primitive::COPPER_178,
    keycap_text: primitive::SAND_780,
    kit_drop_target: Color {
        rgb: 0xF67A40,
        hsla: Hsla {
            h: 0.053113554,
            s: 0.91,
            l: 0.60784316,
            a: 0.12156863,
        },
    },
    kit_scrollbar_thumb: Color {
        rgb: 0xBEB6AB,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.12751679,
            l: 0.7078432,
            a: 0.5019608,
        },
    },
    kit_scrollbar_thumb_hover: Color {
        rgb: 0xBEB6AB,
        hsla: Hsla {
            h: 0.096491225,
            s: 0.12751679,
            l: 0.7078432,
            a: 0.8,
        },
    },
    kit_scrollbar_track: Color {
        rgb: 0x000000,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        },
    },
    kit_status_on: primitive::COPPER_178,
    matrix_missing_text: primitive::SAND_780,
    row_selected_bg: primitive::COPPER_285,
    row_selected_border: primitive::SAND_660,
    row_selected_unfocused_bg: primitive::SAND_290,
    scrim: Color {
        rgb: 0x000000,
        hsla: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.54901963,
        },
    },
    segment_selected_bg: primitive::SAND_262,
    segment_selected_ring: primitive::SAND_660,
    segment_track: primitive::SAND_249,
    switch_off_ring: primitive::SAND_660,
    switch_off_track: primitive::SAND_290,
    switch_on_bg: primitive::SAND_938,
    switch_thumb: primitive::SAND_224,
    tile_bg: primitive::SAND_249,
    tile_glyph: primitive::SAND_780,
    tile_selected_bg: primitive::SAND_224,
    tile_selected_glyph: primitive::SAND_938,
    floating: [
        Shadow {
            offset_y: 18.0,
            blur: 48.0,
            spread: 0.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.6,
                },
            },
            inset: false,
        },
        Shadow {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: Color {
                rgb: 0xFFFFFF,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 1.0,
                    a: 0.101960786,
                },
            },
            inset: false,
        },
    ],
    raised: [
        Shadow {
            offset_y: 4.0,
            blur: 16.0,
            spread: 0.0,
            color: Color {
                rgb: 0x000000,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.4,
                },
            },
            inset: false,
        },
        Shadow {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: Color {
                rgb: 0xFFFFFF,
                hsla: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 1.0,
                    a: 0.078431375,
                },
            },
            inset: true,
        },
    ],
};

impl ColorSet {
    /// The color of a token named as in `design/tokens.json`.
    pub fn by_name(&self, name: &str) -> Option<Color> {
        let color = match name {
            "accent.fill" => self.accent_fill,
            "accent.fill.hover" => self.accent_fill_hover,
            "accent.on" => self.accent_on,
            "accent.text" => self.accent_text,
            "bg.elevated" => self.bg_elevated,
            "bg.hover" => self.bg_hover,
            "bg.keycap" => self.bg_keycap,
            "bg.selected" => self.bg_selected,
            "bg.selected.unfocused" => self.bg_selected_unfocused,
            "bg.sidebar" => self.bg_sidebar,
            "bg.surface" => self.bg_surface,
            "bg.window" => self.bg_window,
            "border.control" => self.border_control,
            "border.keycap" => self.border_keycap,
            "border.separator" => self.border_separator,
            "brand.mark" => self.brand_mark,
            "focus.ring" => self.focus_ring,
            "status.attention.soft" => self.status_attention_soft,
            "status.attention.text" => self.status_attention_text,
            "status.danger.fill" => self.status_danger_fill,
            "status.danger.on" => self.status_danger_on,
            "status.danger.soft" => self.status_danger_soft,
            "status.danger.text" => self.status_danger_text,
            "status.decision.soft" => self.status_decision_soft,
            "status.decision.text" => self.status_decision_text,
            "status.ok.soft" => self.status_ok_soft,
            "status.ok.text" => self.status_ok_text,
            "team.moss" => self.team_moss,
            "team.plum" => self.team_plum,
            "team.slate" => self.team_slate,
            "team.teal" => self.team_teal,
            "text.primary" => self.text_primary,
            "text.secondary" => self.text_secondary,
            "text.tertiary" => self.text_tertiary,
            "clipboard.thread" => self.clipboard_thread,
            "clipboard.thread.end" => self.clipboard_thread_end,
            "otp.ring" => self.otp_ring,
            "otp.ring.warn" => self.otp_ring_warn,
            "otp.track" => self.otp_track,
            "prd.chip.bg" => self.prd_chip_bg,
            "prd.chip.text" => self.prd_chip_text,
            "secret.digit" => self.secret_digit,
            "secret.symbol" => self.secret_symbol,
            "vault.other" => self.vault_other,
            "vault.personal" => self.vault_personal,
            "brand.ferris.cheek" => self.brand_ferris_cheek,
            "brand.ferris.chest" => self.brand_ferris_chest,
            "brand.ferris.claw" => self.brand_ferris_claw,
            "brand.ferris.eye" => self.brand_ferris_eye,
            "brand.ferris.glint" => self.brand_ferris_glint,
            "brand.ferris.key" => self.brand_ferris_key,
            "brand.ferris.metal" => self.brand_ferris_metal,
            "brand.ferris.shell" => self.brand_ferris_shell,
            "brand.icon.bg.bottom" => self.brand_icon_bg_bottom,
            "brand.icon.bg.top" => self.brand_icon_bg_top,
            "brand.icon.cheek" => self.brand_icon_cheek,
            "brand.icon.claw" => self.brand_icon_claw,
            "brand.icon.eye" => self.brand_icon_eye,
            "brand.icon.glint" => self.brand_icon_glint,
            "brand.icon.key" => self.brand_icon_key,
            "brand.icon.shell" => self.brand_icon_shell,
            "banner.attention.bg" => self.banner_attention_bg,
            "banner.attention.glyph" => self.banner_attention_glyph,
            "banner.attention.text" => self.banner_attention_text,
            "banner.danger.bg" => self.banner_danger_bg,
            "banner.danger.glyph" => self.banner_danger_glyph,
            "banner.danger.text" => self.banner_danger_text,
            "banner.decision.bg" => self.banner_decision_bg,
            "banner.decision.glyph" => self.banner_decision_glyph,
            "banner.decision.text" => self.banner_decision_text,
            "banner.ok.bg" => self.banner_ok_bg,
            "banner.ok.glyph" => self.banner_ok_glyph,
            "banner.ok.text" => self.banner_ok_text,
            "button.danger.bg" => self.button_danger_bg,
            "button.danger.hover" => self.button_danger_hover,
            "button.danger.text" => self.button_danger_text,
            "button.disabled.bg" => self.button_disabled_bg,
            "button.disabled.text" => self.button_disabled_text,
            "button.ghost.hover.bg" => self.button_ghost_hover_bg,
            "button.ghost.text" => self.button_ghost_text,
            "button.primary.bg" => self.button_primary_bg,
            "button.primary.hover" => self.button_primary_hover,
            "button.primary.text" => self.button_primary_text,
            "button.quiet.hover.bg" => self.button_quiet_hover_bg,
            "button.quiet.text" => self.button_quiet_text,
            "button.secondary.bg" => self.button_secondary_bg,
            "button.secondary.hover" => self.button_secondary_hover,
            "button.secondary.text" => self.button_secondary_text,
            "chip.neutral.bg" => self.chip_neutral_bg,
            "chip.neutral.text" => self.chip_neutral_text,
            "field.bg" => self.field_bg,
            "field.border" => self.field_border,
            "field.border.error" => self.field_border_error,
            "keycap.bg" => self.keycap_bg,
            "keycap.border" => self.keycap_border,
            "keycap.on_accent.border" => self.keycap_on_accent_border,
            "keycap.on_accent.text" => self.keycap_on_accent_text,
            "keycap.text" => self.keycap_text,
            "kit.drop.target" => self.kit_drop_target,
            "kit.scrollbar.thumb" => self.kit_scrollbar_thumb,
            "kit.scrollbar.thumb.hover" => self.kit_scrollbar_thumb_hover,
            "kit.scrollbar.track" => self.kit_scrollbar_track,
            "kit.status.on" => self.kit_status_on,
            "matrix.missing.text" => self.matrix_missing_text,
            "row.selected.bg" => self.row_selected_bg,
            "row.selected.border" => self.row_selected_border,
            "row.selected.unfocused.bg" => self.row_selected_unfocused_bg,
            "scrim" => self.scrim,
            "segment.selected.bg" => self.segment_selected_bg,
            "segment.selected.ring" => self.segment_selected_ring,
            "segment.track" => self.segment_track,
            "switch.off.ring" => self.switch_off_ring,
            "switch.off.track" => self.switch_off_track,
            "switch.on.bg" => self.switch_on_bg,
            "switch.thumb" => self.switch_thumb,
            "tile.bg" => self.tile_bg,
            "tile.glyph" => self.tile_glyph,
            "tile.selected.bg" => self.tile_selected_bg,
            "tile.selected.glyph" => self.tile_selected_glyph,
            _ => return None,
        };
        Some(color)
    }
}
