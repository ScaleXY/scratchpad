//! Multi-device previews for the Preview tab.
//!
//! iced has no HTML engine, so the text rendering of the email is laid out in
//! "screens" sized like real device viewports and scaled down to fit the pane.
//! Widths, heights, paddings and font sizes are all multiplied by the scale, so
//! lines wrap like they would in the real viewport. "Open in browser" exports
//! the real HTML viewer (with the same devices) for pixel-perfect checks.

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, responsive, row, scrollable, space, Row};
use iced::{Alignment, Element, Fill, Size};

use super::icons::Icon;
use super::style::{self, Kind};
use super::view::{btn, fg, fg2, hairline, icon, icon_btn, muted, muted_c, tx, MEDIUM};
use super::{App, Message};

/// Space around the screens, and between them in "All".
const PAD: f32 = 24.0;
const GAP: f32 = 24.0;
/// The content column inside wide viewports, like a typical email layout.
const CONTENT_MAX: f32 = 680.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Device {
    /// The full width of the pane (no device frame).
    #[default]
    Fit,
    Desktop,
    Laptop,
    Tablet,
    Mobile,
    /// Mobile, tablet and desktop side by side.
    All,
}

impl Device {
    pub const ALL: [Device; 6] = [
        Device::Fit,
        Device::Desktop,
        Device::Laptop,
        Device::Tablet,
        Device::Mobile,
        Device::All,
    ];

    /// The device id understood by the exported HTML viewer.
    pub fn id(self) -> &'static str {
        match self {
            Device::Fit => "fit",
            Device::Desktop => "desktop",
            Device::Laptop => "laptop",
            Device::Tablet => "tablet",
            Device::Mobile => "mobile",
            Device::All => "all",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Device::Fit => "Fit",
            Device::Desktop => "Desktop",
            Device::Laptop => "Laptop",
            Device::Tablet => "Tablet",
            Device::Mobile => "Mobile",
            Device::All => "All",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Device::Fit => Icon::Maximize2,
            Device::Desktop => Icon::Monitor,
            Device::Laptop => Icon::Laptop,
            Device::Tablet => Icon::Tablet,
            Device::Mobile => Icon::Smartphone,
            Device::All => Icon::LayoutGrid,
        }
    }

    /// Tablets and phones can be turned to landscape.
    pub fn rotatable(self) -> bool {
        matches!(self, Device::Tablet | Device::Mobile | Device::All)
    }

    /// The viewport in CSS pixels, for single devices.
    fn viewport(self, landscape: bool) -> Option<(f32, f32)> {
        let (w, h, rotates) = match self {
            Device::Desktop => (1280.0, 800.0, false),
            Device::Laptop => (1024.0, 640.0, false),
            Device::Tablet => (768.0, 1024.0, true),
            Device::Mobile => (390.0, 844.0, true),
            Device::Fit | Device::All => return None,
        };
        Some(if landscape && rotates { (h, w) } else { (w, h) })
    }

    /// The screens shown for this choice.
    fn screens(self) -> Vec<Device> {
        match self {
            Device::Fit => vec![],
            Device::All => vec![Device::Mobile, Device::Tablet, Device::Desktop],
            device => vec![device],
        }
    }
}

/// The screens (device, CSS width, CSS height) for `device` and the scale that fits
/// them side by side into a pane `width` wide. Screens are never scaled up.
pub fn layout(device: Device, landscape: bool, width: f32) -> (Vec<(Device, f32, f32)>, f32) {
    let screens: Vec<(Device, f32, f32)> = device
        .screens()
        .into_iter()
        .filter_map(|d| d.viewport(landscape).map(|(w, h)| (d, w, h)))
        .collect();
    let total = screens.iter().map(|(_, w, _)| w).sum::<f32>()
        + GAP * screens.len().saturating_sub(1) as f32;
    let available = (width - 2.0 * PAD).max(80.0);
    let scale = if total > 0.0 {
        (available / total).min(1.0)
    } else {
        1.0
    };
    (screens, scale)
}

/// The Preview tab: device bar and the scaled screens.
pub fn preview(app: &App) -> Element<'_, Message> {
    responsive(move |size: Size| {
        let (screens, scale) = layout(app.device, app.landscape, size.width);
        let percent = (scale * 100.0).round();
        let hint = match (app.device, screens.first()) {
            (Device::Fit, _) | (_, None) => None,
            (Device::All, _) => Some(format!("{} devices · {percent}%", screens.len())),
            (_, Some((_, w, h))) => Some(format!("{w} × {h} · {percent}%")),
        };

        let area: Element<'_, Message> = if screens.is_empty() {
            fit(app)
        } else {
            let all = app.device == Device::All;
            let row = Row::with_children(
                screens
                    .iter()
                    .map(|(device, w, h)| screen(app, *device, *w, *h, scale, all)),
            )
            .spacing(GAP * scale)
            .align_y(Alignment::Start);
            scrollable(container(row).center_x(Fill).padding(PAD))
                .style(style::scroll)
                .height(Fill)
                .into()
        };

        column![
            container(device_bar(app, size.width, hint)).padding([8, 24]),
            hairline(),
            container(area)
                .style(style::preview_bg)
                .width(Fill)
                .height(Fill),
        ]
        .into()
    })
    .into()
}

/// Device segments, rotate toggle, size hint and "Open in browser". Labels are
/// dropped when the pane gets narrow.
fn device_bar(app: &App, width: f32, hint: Option<String>) -> Element<'_, Message> {
    let labels = width >= 690.0;
    let open_label = width >= 820.0;
    let segments = Row::with_children(Device::ALL.into_iter().map(|device| {
        let on = app.device == device;
        let mut content = row![icon(device.icon(), 14.0, if on { fg } else { muted_c })]
            .spacing(5)
            .align_y(Alignment::Center);
        if labels {
            content = content.push(
                iced::widget::text(device.label())
                    .size(12.5)
                    .font(MEDIUM)
                    .wrapping(Wrapping::None),
            );
        }
        button(content)
            .padding(if labels { [4, 9] } else { [4, 7] })
            .style(style::btn(Kind::Segment(on)))
            .on_press(Message::Device(device))
            .into()
    }))
    .spacing(2);

    let mut bar = row![container(segments).padding(2).style(style::segmented)]
        .spacing(10)
        .align_y(Alignment::Center);
    if app.device.rotatable() {
        bar = bar.push(
            container(
                button(icon(
                    Icon::RotateCcw,
                    14.0,
                    if app.landscape { fg } else { muted_c },
                ))
                .padding([4, 7])
                .style(style::btn(Kind::Segment(app.landscape)))
                .on_press(Message::Rotate),
            )
            .padding(2)
            .style(style::segmented),
        );
    }
    if let Some(hint) = hint.filter(|_| width >= 460.0) {
        bar = bar.push(muted(hint).wrapping(Wrapping::None));
    }
    bar = bar.push(space::horizontal());
    let open: Element<'_, Message> = if open_label {
        btn(
            Some(Icon::ExternalLink),
            "Open in browser",
            Kind::Secondary,
            Some(Message::OpenPreviewInBrowser),
        )
        .into()
    } else {
        icon_btn(Icon::ExternalLink, Message::OpenPreviewInBrowser, false).into()
    };
    bar.push(open).into()
}

/// The full-width "paper" rendering.
fn fit(app: &App) -> Element<'_, Message> {
    let paper = container(
        tx(app.preview_text.as_str(), 13.5, fg2)
            .line_height(1.55)
            .wrapping(Wrapping::WordOrGlyph),
    )
    .padding([28, 32])
    .width(Fill)
    .max_width(CONTENT_MAX)
    .style(style::card);
    scrollable(container(paper).center_x(Fill).padding(PAD))
        .style(style::scroll)
        .height(Fill)
        .into()
}

/// One device screen of `w` × `h` CSS pixels, drawn at `scale`.
fn screen(
    app: &App,
    device: Device,
    w: f32,
    h: f32,
    scale: f32,
    focusable: bool,
) -> Element<'_, Message> {
    let s = scale;
    let gutter = if w < 600.0 { 16.0 } else { 32.0 };
    let body = tx(app.preview_text.as_str(), 14.0 * s, fg2)
        .line_height(1.55)
        .wrapping(Wrapping::WordOrGlyph);
    let content = container(container(body).width(Fill).max_width(CONTENT_MAX * s))
        .center_x(Fill)
        .padding(gutter * s);
    let radius = match device {
        Device::Mobile => (40.0 * s).max(8.0),
        Device::Tablet => (22.0 * s).max(6.0),
        _ => (10.0 * s).max(4.0),
    };
    let viewport = container(
        scrollable(content)
            .direction(Direction::Vertical(
                Scrollbar::new().width(4).scroller_width(4).margin(2),
            ))
            .style(style::scroll)
            .width(Fill)
            .height(Fill),
    )
    .width(w * s)
    .height(h * s)
    .clip(true)
    .style(style::screen(radius));

    // Two short lines, so captions fit above small screens in "All".
    let caption = column![
        row![
            icon(device.icon(), 13.0, muted_c),
            tx(device.label(), 12.0, fg).font(MEDIUM)
        ]
        .spacing(5)
        .align_y(Alignment::Center),
        muted(format!("{w} × {h}"))
            .size(11)
            .wrapping(Wrapping::None),
    ]
    .spacing(1)
    .align_x(Alignment::Center);
    let caption: Element<'_, Message> = if focusable {
        button(caption)
            .padding([2, 6])
            .style(style::btn(Kind::Ghost))
            .on_press(Message::Device(device))
            .into()
    } else {
        container(caption).padding([2, 6]).into()
    };

    column![caption, viewport]
        .spacing(8)
        .align_x(Alignment::Center)
        .into()
}
