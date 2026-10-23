//! Design tokens and widget styles. Mirrors the web interface's palette:
//! a neutral light theme and a pure black (#000000) dark theme.

use iced::theme::palette::{self, Extended, Pair};
use iced::widget::{
    button, checkbox, container, pick_list, scrollable, svg, text_editor, text_input, toggler,
};
use iced::{border, Background, Border, Color, Shadow, Theme, Vector};

/// Semantic colors for one theme.
pub struct Tokens {
    pub bg: Color,
    pub sidebar: Color,
    pub surface: Color,
    pub surface_2: Color,
    pub hover: Color,
    pub active: Color,
    pub border: Color,
    pub border_strong: Color,
    pub fg: Color,
    pub fg_2: Color,
    pub muted: Color,
    pub faint: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_soft: Color,
    pub ok: Color,
    pub ok_soft: Color,
    pub warn: Color,
    pub warn_soft: Color,
    pub danger: Color,
    pub danger_soft: Color,
    pub preview: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const fn rgba(hex: u32, alpha: f32) -> Color {
    Color::from_rgba8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, alpha)
}

pub const DARK: Tokens = Tokens {
    bg: rgb(0x000000),
    sidebar: rgb(0x000000),
    surface: rgb(0x0a0a0a),
    surface_2: rgb(0x111111),
    hover: rgb(0x141414),
    active: rgb(0x1c1c1c),
    border: rgb(0x1f1f1f),
    border_strong: rgb(0x2e2e2e),
    fg: rgb(0xededed),
    fg_2: rgb(0xc7c7c7),
    muted: rgb(0x8f8f8f),
    faint: rgb(0x7d7d7d),
    accent: rgb(0x7c7cff),
    accent_hover: rgb(0x9393ff),
    accent_soft: rgba(0x7c7cff, 0.14),
    ok: rgb(0x3dd68c),
    ok_soft: rgba(0x3dd68c, 0.12),
    warn: rgb(0xf5b041),
    warn_soft: rgba(0xf5b041, 0.13),
    danger: rgb(0xff6369),
    danger_soft: rgba(0xff6369, 0.13),
    preview: rgb(0x0a0a0a),
};

pub const LIGHT: Tokens = Tokens {
    bg: rgb(0xffffff),
    sidebar: rgb(0xfafafa),
    surface: rgb(0xffffff),
    surface_2: rgb(0xf5f5f5),
    hover: rgb(0xf2f2f2),
    active: rgb(0xebebeb),
    border: rgb(0xebebeb),
    border_strong: rgb(0xdcdcdc),
    fg: rgb(0x0a0a0a),
    fg_2: rgb(0x404040),
    muted: rgb(0x666666),
    faint: rgb(0x707070),
    accent: rgb(0x5b5bf0),
    accent_hover: rgb(0x4b4be0),
    accent_soft: rgba(0x5b5bf0, 0.09),
    ok: rgb(0x16a34a),
    ok_soft: rgba(0x16a34a, 0.10),
    warn: rgb(0xc2770c),
    warn_soft: rgba(0xd97706, 0.12),
    danger: rgb(0xe5484d),
    danger_soft: rgba(0xe5484d, 0.10),
    preview: rgb(0xf4f4f5),
};

pub fn tokens(theme: &Theme) -> &'static Tokens {
    if theme.extended_palette().is_dark {
        &DARK
    } else {
        &LIGHT
    }
}

fn build_theme(name: &'static str, t: &'static Tokens, dark: bool) -> Theme {
    let palette = iced::theme::Palette {
        background: t.bg,
        text: t.fg,
        primary: t.accent,
        success: t.ok,
        warning: t.warn,
        danger: t.danger,
    };
    Theme::custom_with_fn(name, palette, move |palette| {
        let mut extended = Extended::generate(palette);
        // Pin the neutral ramp so built-in widgets match the design tokens exactly.
        extended.background = palette::Background {
            base: Pair::new(t.bg, t.fg),
            weakest: Pair::new(t.surface, t.fg),
            weaker: Pair::new(t.surface_2, t.fg),
            weak: Pair::new(t.hover, t.fg),
            neutral: Pair::new(t.border, t.fg),
            strong: Pair::new(t.border_strong, t.fg),
            stronger: Pair::new(t.faint, t.fg),
            strongest: Pair::new(t.fg, t.bg),
        };
        extended.is_dark = dark;
        extended
    })
}

pub fn dark_theme() -> Theme {
    build_theme("Scratchpad Dark", &DARK, true)
}

pub fn light_theme() -> Theme {
    build_theme("Scratchpad Light", &LIGHT, false)
}

// ---------------------------------------------------------------- containers

fn panel(background: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(background)),
        ..Default::default()
    }
}

pub fn app_bg(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        text_color: Some(t.fg),
        ..panel(t.bg)
    }
}

pub fn sidebar(theme: &Theme) -> container::Style {
    panel(tokens(theme).sidebar)
}

pub fn preview_bg(theme: &Theme) -> container::Style {
    panel(tokens(theme).preview)
}

pub fn divider(theme: &Theme) -> container::Style {
    panel(tokens(theme).border)
}

pub fn card(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.surface)),
        border: Border {
            color: t.border,
            width: 1.0,
            radius: 12.0.into(),
        },
        ..Default::default()
    }
}

pub fn code_block(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.surface_2)),
        border: Border {
            color: t.border,
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: Some(t.fg_2),
        ..Default::default()
    }
}

pub fn hero(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Gradient(
            iced::gradient::Linear::new(std::f32::consts::FRAC_PI_4 * 3.0)
                .add_stop(0.0, t.accent_soft)
                .add_stop(0.7, t.surface)
                .into(),
        )),
        border: Border {
            color: t.border,
            width: 1.0,
            radius: 12.0.into(),
        },
        ..Default::default()
    }
}

pub fn tile(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.surface_2)),
        border: Border {
            color: t.border,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..Default::default()
    }
}

pub fn segmented(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.surface_2)),
        border: Border {
            color: t.border,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// A device screen in the Preview tab.
pub fn screen(radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let t = tokens(theme);
        container::Style {
            background: Some(Background::Color(t.surface)),
            text_color: Some(t.fg_2),
            border: Border {
                color: t.border_strong,
                width: 1.0,
                radius: radius.into(),
            },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.18),
                offset: Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..Default::default()
        }
    }
}

/// The dimmed layer behind a modal dialog.
pub fn backdrop(theme: &Theme) -> container::Style {
    let dark = theme.extended_palette().is_dark;
    panel(Color::from_rgba(
        0.0,
        0.0,
        0.0,
        if dark { 0.7 } else { 0.4 },
    ))
}

pub fn dialog(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.surface)),
        text_color: Some(t.fg),
        border: Border {
            color: t.border_strong,
            width: 1.0,
            radius: 14.0.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
            offset: Vector::new(0.0, 16.0),
            blur_radius: 48.0,
        },
        ..Default::default()
    }
}

pub fn avatar(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
        text_color: Some(Color::WHITE),
        border: border::rounded(100),
        ..Default::default()
    }
}

pub fn dot(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
        border: border::rounded(10),
        ..Default::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Ok,
    Warn,
    Bad,
    Solid,
}

pub fn tone_colors(t: &Tokens, tone: Tone) -> (Color, Color, Option<Color>) {
    match tone {
        Tone::Neutral => (t.surface_2, t.muted, Some(t.border)),
        Tone::Ok => (t.ok_soft, t.ok, None),
        Tone::Warn => (t.warn_soft, t.warn, None),
        Tone::Bad => (t.danger_soft, t.danger, None),
        Tone::Solid => (t.accent, Color::WHITE, None),
    }
}

pub fn chip(tone: Tone) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let (bg, fg, edge) = tone_colors(tokens(theme), tone);
        container::Style {
            background: Some(Background::Color(bg)),
            text_color: Some(fg),
            border: Border {
                color: edge.unwrap_or(Color::TRANSPARENT),
                width: if edge.is_some() { 1.0 } else { 0.0 },
                radius: if tone == Tone::Solid {
                    9.0.into()
                } else {
                    6.0.into()
                },
            },
            ..Default::default()
        }
    }
}

pub fn note(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.warn_soft)),
        text_color: Some(t.warn),
        border: border::rounded(8),
        ..Default::default()
    }
}

pub fn toast(error: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let t = tokens(theme);
        let (bg, fg) = if error {
            (t.danger, Color::WHITE)
        } else {
            (t.fg, t.bg)
        };
        container::Style {
            background: Some(Background::Color(bg)),
            text_color: Some(fg),
            border: border::rounded(10),
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
                offset: Vector::new(0.0, 8.0),
                blur_radius: 24.0,
            },
            ..Default::default()
        }
    }
}

pub fn tooltip(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.fg)),
        text_color: Some(t.bg),
        border: border::rounded(6),
        ..Default::default()
    }
}

pub fn confirm_bar(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.danger_soft)),
        text_color: Some(t.fg),
        border: Border {
            color: t.border,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------- buttons

fn hovered(status: button::Status) -> bool {
    matches!(status, button::Status::Hovered | button::Status::Pressed)
}

/// Sidebar entries.
pub fn nav(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let background = if active {
            Some(Background::Color(t.active))
        } else if hovered(status) {
            Some(Background::Color(t.hover))
        } else {
            None
        };
        button::Style {
            background,
            text_color: if active || hovered(status) {
                t.fg
            } else {
                t.fg_2
            },
            border: border::rounded(6),
            ..Default::default()
        }
    }
}

/// Email list rows.
pub fn row(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let background = if active {
            Some(Background::Color(t.accent_soft))
        } else if hovered(status) {
            Some(Background::Color(t.hover))
        } else {
            None
        };
        button::Style {
            background,
            text_color: t.fg,
            border: border::rounded(8),
            ..Default::default()
        }
    }
}

/// Underlined tabs.
pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        button::Style {
            background: None,
            text_color: if active || hovered(status) {
                t.fg
            } else {
                t.muted
            },
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
    Icon,
    IconDanger,
    Link,
    Segment(bool),
}

pub fn btn(kind: Kind) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let hover = hovered(status);
        let disabled = matches!(status, button::Status::Disabled);
        let mut style = match kind {
            Kind::Primary => button::Style {
                background: Some(Background::Color(if hover {
                    t.accent_hover
                } else {
                    t.accent
                })),
                text_color: Color::WHITE,
                border: border::rounded(6),
                ..Default::default()
            },
            Kind::Secondary => button::Style {
                background: Some(Background::Color(if hover { t.hover } else { t.surface })),
                text_color: t.fg,
                border: Border {
                    color: t.border_strong,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            },
            Kind::Ghost | Kind::Icon => button::Style {
                background: hover.then_some(Background::Color(t.hover)),
                text_color: if hover {
                    t.fg
                } else if kind == Kind::Icon {
                    t.muted
                } else {
                    t.fg_2
                },
                border: border::rounded(6),
                ..Default::default()
            },
            Kind::Danger | Kind::IconDanger => button::Style {
                background: hover.then_some(Background::Color(t.danger_soft)),
                text_color: if hover || kind == Kind::Danger {
                    t.danger
                } else {
                    t.muted
                },
                border: border::rounded(6),
                ..Default::default()
            },
            Kind::Link => button::Style {
                background: None,
                text_color: if hover { t.fg } else { t.muted },
                ..Default::default()
            },
            Kind::Segment(on) => button::Style {
                background: on.then_some(Background::Color(if theme.extended_palette().is_dark {
                    t.active
                } else {
                    t.surface
                })),
                text_color: if on || hover { t.fg } else { t.muted },
                border: Border {
                    color: if on && !theme.extended_palette().is_dark {
                        t.border
                    } else {
                        Color::TRANSPARENT
                    },
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            },
        };
        if disabled {
            style.text_color = style.text_color.scale_alpha(0.5);
        }
        style
    }
}

/// Tints a monochrome icon with the button's text color.
pub fn icon_color(color: fn(&Tokens) -> Color) -> impl Fn(&Theme, svg::Status) -> svg::Style {
    move |theme, _| svg::Style {
        color: Some(color(tokens(theme))),
    }
}

// ---------------------------------------------------------------- inputs

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let t = tokens(theme);
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(if focused { t.bg } else { t.surface_2 }),
        border: Border {
            color: if focused {
                t.accent
            } else if matches!(status, text_input::Status::Hovered) {
                t.border_strong
            } else {
                t.border
            },
            width: 1.0,
            radius: 8.0.into(),
        },
        icon: t.faint,
        placeholder: t.faint,
        value: t.fg,
        selection: t.accent_soft,
    }
}

pub fn field(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let t = tokens(theme);
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(t.bg),
        border: Border {
            color: if focused { t.accent } else { t.border_strong },
            width: 1.0,
            radius: 6.0.into(),
        },
        ..input(theme, status)
    }
}

pub fn select(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let t = tokens(theme);
    pick_list::Style {
        text_color: t.fg,
        placeholder_color: t.faint,
        handle_color: t.muted,
        background: Background::Color(t.bg),
        border: Border {
            color: if matches!(status, pick_list::Status::Opened { .. }) {
                t.accent
            } else {
                t.border_strong
            },
            width: 1.0,
            radius: 6.0.into(),
        },
    }
}

pub fn switch(theme: &Theme, status: toggler::Status) -> toggler::Style {
    let t = tokens(theme);
    let on = match status {
        toggler::Status::Active { is_toggled }
        | toggler::Status::Hovered { is_toggled }
        | toggler::Status::Disabled { is_toggled } => is_toggled,
    };
    toggler::Style {
        background: Background::Color(if on { t.accent } else { t.border_strong }),
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: Background::Color(Color::WHITE),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: Some(t.fg),
        border_radius: None,
        padding_ratio: 0.15,
    }
}

pub fn check(theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let t = tokens(theme);
    let checked = match status {
        checkbox::Status::Active { is_checked }
        | checkbox::Status::Hovered { is_checked }
        | checkbox::Status::Disabled { is_checked } => is_checked,
    };
    checkbox::Style {
        background: Background::Color(if checked { t.accent } else { t.surface }),
        icon_color: Color::WHITE,
        border: Border {
            color: if checked { t.accent } else { t.border_strong },
            width: 1.5,
            radius: 4.0.into(),
        },
        text_color: Some(t.muted),
    }
}

pub fn code_editor(theme: &Theme, _status: text_editor::Status) -> text_editor::Style {
    let t = tokens(theme);
    text_editor::Style {
        background: Background::Color(t.bg),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        placeholder: t.faint,
        value: t.fg_2,
        selection: t.accent_soft,
    }
}

pub fn scroll(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let t = tokens(theme);
    let mut style = scrollable::default(theme, status);
    let hovered = matches!(
        status,
        scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. }
    );
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: Background::Color(if hovered { t.border_strong } else { t.border }),
            border: border::rounded(4),
        },
    };
    style.container = container::Style::default();
    style.vertical_rail = rail;
    style.horizontal_rail = rail;
    style
}
