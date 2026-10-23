//! View functions for the desktop GUI.

use iced::font::Weight;
use iced::widget::{
    button, checkbox, column, container, hover, pick_list, row, scrollable, space, stack, svg,
    text, text_editor, text_input, toggler, tooltip, Column, Row,
};
use iced::{Alignment, Color, Element, Fill, Font, Length, Theme};
use scratchpad_core::{
    EmailSummary, ForwardEncryption, LinkStatus, MailboxKind, RemoteSource, ThemePreference,
};

use super::forms::{Field, RemoteField, RemoteKind, RemoteMsg, SettingsMsg, Toggle};
use super::icons::{Icon, APP_LOGO};
use super::style::{self, Kind, Tokens, Tone};
use super::{cloud, preview, App, Confirm, Message, Page, Tab};

/// Space reserved for the transparent macOS title bar (traffic lights).
pub const TOP_INSET: f32 = if cfg!(target_os = "macos") { 28.0 } else { 0.0 };

/// Widget ids.
pub const SEARCH: &str = "search";
pub const LIST: &str = "email-list";

/// Email list rows all have the same height, so the list only builds the rows
/// that are on screen and keyboard navigation can scroll exactly.
pub const ROW_H: f32 = 76.0;
pub const ROW_STRIDE: f32 = ROW_H + 2.0;
const LIST_PAD: f32 = 6.0;
/// Rows built above and below the visible ones.
const OVERSCAN: usize = 8;
/// Approximate height of everything above the list (title, search), used before
/// the list reports its real height.
pub const LIST_CHROME: f32 = 110.0 + TOP_INSET;

const SIDEBAR_W: f32 = 240.0;
/// Longer subjects are cut in the detail heading (the details grid shows all of it).
const SUBJECT_MAX: usize = 150;

fn list_width(app: &App) -> f32 {
    if app.window.width < 1100.0 {
        320.0
    } else {
        380.0
    }
}

/// Width of the detail pane.
fn detail_width(app: &App) -> f32 {
    app.window.width - SIDEBAR_W - list_width(app) - 2.0
}

/// Rows of a list of `count` emails to build at scroll offset `scroll` in a
/// viewport `height` tall.
pub fn visible_rows(count: usize, scroll: f32, height: f32) -> std::ops::Range<usize> {
    let total = count as f32 * ROW_STRIDE + 2.0 * LIST_PAD;
    // The list may have shrunk since the last scroll event.
    let scroll = scroll.min((total - height).max(0.0)).max(0.0);
    let first = ((scroll - LIST_PAD) / ROW_STRIDE).floor().max(0.0) as usize;
    let visible = (height / ROW_STRIDE).ceil() as usize + 1;
    let start = first.saturating_sub(OVERSCAN).min(count);
    let end = (first + visible + OVERSCAN).min(count);
    start..end
}

/// The scroll offset that brings row `index` into view, if it isn't already.
pub fn reveal_offset(index: usize, scroll: f32, height: f32) -> Option<f32> {
    let top = LIST_PAD + index as f32 * ROW_STRIDE;
    let bottom = top + ROW_H;
    if top < scroll {
        Some((top - LIST_PAD).max(0.0))
    } else if bottom > scroll + height {
        Some((bottom + LIST_PAD - height).max(0.0))
    } else {
        None
    }
}

/// The visible height of the email list.
fn list_height(app: &App) -> f32 {
    if app.list_height > 0.0 {
        app.list_height
    } else {
        (app.window.height - LIST_CHROME).max(ROW_STRIDE)
    }
}

/// Space for the macOS title bar, unless the confirm bar already covers it.
fn inset(app: &App) -> f32 {
    if app.confirm.is_some() {
        0.0
    } else {
        TOP_INSET
    }
}

/// At most `max` characters, with an ellipsis when cut.
pub(super) fn ellipsize(value: &str, max: usize) -> String {
    match value.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", value[..end].trim_end()),
        None => value.to_string(),
    }
}

/// One line of text, clipped at the available width (iced has no text ellipsis).
fn one_line<'a>(content: text::Text<'a>) -> Element<'a, Message> {
    container(content.wrapping(text::Wrapping::None))
        .width(Fill)
        .clip(true)
        .into()
}

/// Inter (SIL OFL 1.1) is bundled so the UI looks the same on every platform.
pub const INTER: Font = Font::with_name("Inter");
pub(super) const MONO: Font = Font::MONOSPACE;
pub(super) const MEDIUM: Font = Font {
    weight: Weight::Medium,
    ..INTER
};
pub(super) const SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..INTER
};
pub(super) const BOLD: Font = Font {
    weight: Weight::Bold,
    ..INTER
};

pub fn app_settings() -> iced::Settings {
    iced::Settings {
        default_text_size: 13.into(),
        default_font: INTER,
        fonts: vec![
            include_bytes!("../../../../assets/fonts/Inter-400.ttf")
                .as_slice()
                .into(),
            include_bytes!("../../../../assets/fonts/Inter-500.ttf")
                .as_slice()
                .into(),
            include_bytes!("../../../../assets/fonts/Inter-600.ttf")
                .as_slice()
                .into(),
            include_bytes!("../../../../assets/fonts/Inter-700.ttf")
                .as_slice()
                .into(),
        ],
        ..Default::default()
    }
}

// ---------------------------------------------------------------- primitives

pub(super) type Shade = fn(&Tokens) -> Color;

pub(super) fn fg(t: &Tokens) -> Color {
    t.fg
}
pub(super) fn fg2(t: &Tokens) -> Color {
    t.fg_2
}
pub(super) fn muted_c(t: &Tokens) -> Color {
    t.muted
}
pub(super) fn faint_c(t: &Tokens) -> Color {
    t.faint
}
pub(super) fn accent_c(t: &Tokens) -> Color {
    t.accent
}
pub(super) fn danger_c(t: &Tokens) -> Color {
    t.danger
}

pub(super) fn tx<'a>(
    content: impl text::IntoFragment<'a>,
    size: f32,
    shade: Shade,
) -> text::Text<'a> {
    text(content)
        .size(size)
        .style(move |theme: &Theme| text::Style {
            color: Some(shade(style::tokens(theme))),
        })
}

pub(super) fn muted<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    tx(content, 12.0, muted_c)
}

pub(super) fn faint<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    tx(content, 11.5, faint_c)
}

pub(super) fn icon<'a>(icon: Icon, size: f32, shade: Shade) -> Element<'a, Message> {
    svg(icon.handle())
        .width(size)
        .height(size)
        .style(style::icon_color(shade))
        .into()
}

pub(super) fn hairline<'a>() -> Element<'a, Message> {
    container(space::horizontal().height(1))
        .width(Fill)
        .height(1)
        .style(style::divider)
        .into()
}

pub(super) fn vline<'a>() -> Element<'a, Message> {
    container(space::vertical().width(1))
        .width(1)
        .height(Fill)
        .style(style::divider)
        .into()
}

pub(super) fn chip<'a>(label: impl text::IntoFragment<'a>, tone: Tone) -> Element<'a, Message> {
    container(text(label).size(11).font(MEDIUM))
        .padding([2, 7])
        .style(style::chip(tone))
        .into()
}

pub(super) fn chip_icon<'a>(
    glyph: Icon,
    label: impl text::IntoFragment<'a>,
    tone: Tone,
) -> Element<'a, Message> {
    let shade: Shade = match tone {
        Tone::Ok => |t| t.ok,
        Tone::Warn => |t| t.warn,
        Tone::Bad => |t| t.danger,
        Tone::Solid => |_| Color::WHITE,
        Tone::Neutral => muted_c,
    };
    container(
        row![icon(glyph, 12.0, shade), text(label).size(11).font(MEDIUM)]
            .spacing(4)
            .align_y(Alignment::Center),
    )
    .padding([2, 7])
    .style(style::chip(tone))
    .into()
}

/// A text (+ optional leading icon) button in one of the shared styles.
pub(super) fn btn<'a>(
    glyph: Option<Icon>,
    label: &'a str,
    kind: Kind,
    message: Option<Message>,
) -> button::Button<'a, Message> {
    let shade: Shade = match kind {
        Kind::Primary => |_| Color::WHITE,
        Kind::Danger => danger_c,
        _ => muted_c,
    };
    let mut content = Row::new().spacing(6).align_y(Alignment::Center);
    if let Some(glyph) = glyph {
        content = content.push(icon(glyph, 14.0, shade));
    }
    content = content.push(text(label).size(12.5).font(MEDIUM));
    button(content)
        .padding([6, 11])
        .style(style::btn(kind))
        .on_press_maybe(message)
}

/// A tooltip below `content`.
pub(super) fn tip<'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(label).size(12))
            .padding([4, 8])
            .style(style::tooltip),
        tooltip::Position::Bottom,
    )
    .gap(4)
    .into()
}

pub(super) fn icon_btn<'a>(
    glyph: Icon,
    message: Message,
    danger: bool,
) -> button::Button<'a, Message> {
    button(icon(glyph, 16.0, muted_c))
        .padding(6)
        .style(style::btn(if danger {
            Kind::IconDanger
        } else {
            Kind::Icon
        }))
        .on_press(message)
}

const AVATAR_COLORS: [u32; 10] = [
    0x6366f1, 0x8b5cf6, 0xec4899, 0xf43f5e, 0xf97316, 0xeab308, 0x10b981, 0x14b8a6, 0x0ea5e9,
    0x3b82f6,
];

fn avatar<'a>(name: &str, size: f32) -> Element<'a, Message> {
    let clean: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let parts: Vec<&str> = clean.split_whitespace().collect();
    let mut initials: String = parts
        .first()
        .and_then(|p| p.chars().next())
        .map(String::from)
        .unwrap_or_else(|| "?".into());
    if parts.len() > 1 {
        initials.extend(parts[parts.len() - 1].chars().next());
    }
    let hash = clean
        .chars()
        .fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32));
    let hex = AVATAR_COLORS[(hash % AVATAR_COLORS.len() as u32) as usize];
    let color = Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
    container(
        text(initials.to_uppercase())
            .size(size * 0.38)
            .font(SEMIBOLD),
    )
    .width(size)
    .height(size)
    .center_x(size)
    .center_y(size)
    .style(style::avatar(color))
    .into()
}

pub(super) fn empty<'a>(
    glyph: Icon,
    title: &'a str,
    body: impl text::IntoFragment<'a>,
    action: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut content = column![
        container(icon(glyph, 26.0, faint_c))
            .width(52)
            .height(52)
            .center_x(52)
            .center_y(52)
            .style(style::tile),
        space::vertical().height(4),
        tx(title, 14.0, fg).font(SEMIBOLD),
        tx(body, 12.5, muted_c).align_x(Alignment::Center),
    ]
    .spacing(6)
    .align_x(Alignment::Center)
    .max_width(320);
    if let Some(action) = action {
        content = content.push(space::vertical().height(6)).push(action);
    }
    container(content).center(Fill).padding(24).into()
}

pub(super) fn format_time(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(time) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    let now = Local::now();
    let minutes = (now - time).num_minutes();
    if minutes < 1 {
        "now".into()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if time.date_naive() == now.date_naive() {
        time.format("%H:%M").to_string()
    } else if minutes < 6 * 24 * 60 {
        time.format("%a").to_string()
    } else {
        time.format("%b %d").to_string()
    }
}

pub(super) fn format_full(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%b %d, %Y, %H:%M").to_string())
        .unwrap_or_default()
}

fn format_rfc3339(value: &str) -> Option<String> {
    let time = chrono::DateTime::parse_from_rfc3339(value).ok()?;
    Some(
        time.with_timezone(&chrono::Local)
            .format("%b %d, %Y, %H:%M")
            .to_string(),
    )
}

pub(super) fn format_size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / 1024.0 / 1024.0),
    }
}

fn spam_tone(score: f64, threshold: f64) -> Tone {
    if score >= threshold {
        Tone::Bad
    } else if score >= threshold / 2.0 {
        Tone::Warn
    } else {
        Tone::Ok
    }
}

/// Splits `Name <email>` into its parts.
/// The first `max` items, then "and N more".
fn list_some(items: &[String], max: usize) -> String {
    let mut line = items
        .iter()
        .take(max)
        .map(|item| ellipsize(item, 60))
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > max {
        line.push_str(&format!(" and {} more", items.len() - max));
    }
    line
}

fn split_from(value: &str) -> (String, String) {
    match (value.rfind('<'), value.ends_with('>')) {
        (Some(start), true) => {
            let name = value[..start].trim().trim_matches('"').to_string();
            let email = value[start + 1..value.len() - 1].to_string();
            (if name.is_empty() { email.clone() } else { name }, email)
        }
        _ => (
            if value.is_empty() {
                "Unknown".into()
            } else {
                value.to_string()
            },
            String::new(),
        ),
    }
}

// ---------------------------------------------------------------- root

pub fn view(app: &App) -> Element<'_, Message> {
    let body: Element<'_, Message> = match app.page {
        Page::Inbox => row![email_list(app), vline(), detail(app)]
            .height(Fill)
            .into(),
        Page::Setup => page(app, setup(app)),
        Page::Shares => page(app, shares(app)),
        Page::Cloud => page(app, cloud::cloud_page(app)),
        Page::Remotes => page(app, remotes(app)),
        Page::Settings => page(app, settings(app)),
    };
    let main = container(
        row![
            container(sidebar(app))
                .width(SIDEBAR_W)
                .height(Fill)
                .style(style::sidebar),
            vline(),
            column![confirm_bar(app), body].width(Fill)
        ]
        .height(Fill),
    )
    .style(style::app_bg);

    let mut layers = stack![main];
    if let Some(dialog) = &app.share_dialog {
        layers = layers.push(cloud::share_dialog(app, dialog));
    }
    match &app.toast {
        Some(toast) => {
            let error = toast.error;
            layers
                .push(
                    container(
                        container(
                            row![
                                svg(if error {
                                    Icon::CircleAlert
                                } else {
                                    Icon::Check
                                }
                                .handle())
                                .width(14)
                                .height(14)
                                .style(move |theme: &Theme, _| svg::Style {
                                    color: Some(if error {
                                        Color::WHITE
                                    } else {
                                        style::tokens(theme).bg
                                    })
                                }),
                                text(ellipsize(&toast.message, 400)).size(12.5).font(MEDIUM)
                            ]
                            .spacing(8)
                            .align_y(Alignment::Center),
                        )
                        .max_width(560)
                        .padding([9, 14])
                        .style(style::toast(error)),
                    )
                    .width(Fill)
                    .height(Fill)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::End)
                    .padding(20),
                )
                .into()
        }
        None => layers.into(),
    }
}

fn page<'a>(app: &App, content: Element<'a, Message>) -> Element<'a, Message> {
    scrollable(
        container(container(content).max_width(780).padding([36, 40]))
            .center_x(Fill)
            .padding(iced::Padding {
                top: inset(app),
                ..Default::default()
            }),
    )
    .style(style::scroll)
    .height(Fill)
    .width(Fill)
    .into()
}

pub(super) fn page_head<'a>(
    title: &'a str,
    lead: impl text::IntoFragment<'a>,
) -> Element<'a, Message> {
    column![tx(title, 22.0, fg).font(BOLD), tx(lead, 13.0, muted_c)]
        .spacing(6)
        .into()
}

// ---------------------------------------------------------------- sidebar

fn sidebar(app: &App) -> Element<'_, Message> {
    let trailing = |unread: u64, total: Option<u64>| -> Element<'_, Message> {
        if unread > 0 {
            container(text(unread.to_string()).size(10.5).font(SEMIBOLD))
                .padding([1, 6])
                .style(style::chip(Tone::Solid))
                .into()
        } else if let Some(total) = total.filter(|n| *n > 0) {
            faint(total.to_string()).size(11).into()
        } else {
            space::horizontal().width(0).into()
        }
    };
    let entry = |glyph: Icon,
                 label: String,
                 unread: u64,
                 total: Option<u64>,
                 active: bool,
                 message: Message| {
        let shade: Shade = if active { accent_c } else { muted_c };
        button(
            row![
                icon(glyph, 16.0, shade),
                text(label)
                    .size(13)
                    .font(if active { MEDIUM } else { INTER })
                    .width(Fill),
                trailing(unread, total)
            ]
            .spacing(9)
            .align_y(Alignment::Center),
        )
        .width(Fill)
        .padding([6, 8])
        .style(style::nav(active))
        .on_press(message)
    };
    let label = |title: &'static str| container(faint(title).font(MEDIUM)).padding([4, 8]);

    let total_unread: u64 = app.mailboxes.iter().map(|m| m.unread).sum();
    let total: u64 = app.mailboxes.iter().map(|m| m.total).sum();
    let mut local = Column::new().spacing(1);
    let mut remote = Column::new().spacing(1);
    for mailbox in &app.mailboxes {
        let active =
            app.page == Page::Inbox && app.mailbox.as_deref() == Some(mailbox.name.as_str());
        let glyph = if mailbox.kind == MailboxKind::Remote {
            Icon::CloudDownload
        } else {
            Icon::Mail
        };
        let item = entry(
            glyph,
            ellipsize(&mailbox.name, 40),
            mailbox.unread,
            Some(mailbox.total),
            active,
            Message::SelectMailbox(Some(mailbox.name.clone())),
        );
        match mailbox.kind {
            MailboxKind::Local => local = local.push(item),
            MailboxKind::Remote => remote = remote.push(item),
        }
    }
    if app.mailboxes.iter().all(|m| m.kind != MailboxKind::Local) {
        local = local.push(
            container(faint("Mailboxes appear automatically when mail arrives.").size(12))
                .padding([2, 8]),
        );
    }
    if app.mailboxes.iter().all(|m| m.kind != MailboxKind::Remote) {
        remote = remote.push(
            container(faint("Pull mail from IMAP or a Scratchpad server.").size(12))
                .padding([2, 8]),
        );
    }
    let nav = |glyph: Icon, title: &'static str, page: Page| {
        entry(
            glyph,
            title.to_string(),
            0,
            None,
            app.page == page,
            Message::Page(page),
        )
    };

    let status = &app.status;
    let smtp: Element<'_, Message> = match &status.smtp_error {
        Some(error) => row![
            container(space::horizontal().width(7).height(7)).style(style::dot(style::DARK.danger)),
            tx(error.clone(), 11.5, danger_c)
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into(),
        None => row![
            container(space::horizontal().width(7).height(7)).style(style::dot(style::DARK.ok)),
            tx(format!("smtp://{}", status.smtp_address), 11.5, muted_c).font(MONO)
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into(),
    };
    let theme_button = |pref: ThemePreference, glyph: Icon| {
        let on = app.settings.theme == pref;
        button(icon(glyph, 14.0, if on { fg } else { muted_c }))
            .padding([4, 6])
            .style(style::btn(Kind::Segment(on)))
            .on_press(Message::SetTheme(pref))
    };
    let version: Element<'_, Message> = match &status.web_url {
        Some(url) => button(
            row![
                faint(format!("v{}", status.version)),
                icon(Icon::Globe, 12.0, faint_c),
                faint("Web")
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding(0)
        .style(style::btn(Kind::Link))
        .on_press(Message::OpenUrl(url.clone()))
        .into(),
        None => faint(format!("v{}", status.version)).into(),
    };
    let footer = column![
        smtp,
        row![
            version,
            space::horizontal(),
            container(
                row![
                    theme_button(ThemePreference::System, Icon::Monitor),
                    theme_button(ThemePreference::Light, Icon::Sun),
                    theme_button(ThemePreference::Dark, Icon::Moon)
                ]
                .spacing(2)
            )
            .padding(2)
            .style(style::segmented)
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .padding([10, 12]);

    column![
        space::vertical().height(TOP_INSET),
        row![
            svg(svg::Handle::from_memory(APP_LOGO)).width(28).height(28),
            column![
                tx("Scratchpad", 14.0, fg).font(BOLD),
                faint("Email testing")
            ]
            .spacing(0)
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .padding([10, 16]),
        scrollable(
            column![
                entry(
                    Icon::Inbox,
                    "All emails".into(),
                    total_unread,
                    Some(total),
                    app.page == Page::Inbox && app.mailbox.is_none(),
                    Message::SelectMailbox(None)
                ),
                space::vertical().height(10),
                label("Mailboxes"),
                local,
                space::vertical().height(10),
                row![
                    label("Remote"),
                    space::horizontal(),
                    button(icon(Icon::Plus, 14.0, muted_c))
                        .padding(3)
                        .style(style::btn(Kind::Icon))
                        .on_press(Message::Page(Page::Remotes))
                ]
                .align_y(Alignment::Center),
                remote,
                space::vertical().height(10),
                label("App"),
                nav(Icon::Terminal, "Setup", Page::Setup),
                nav(Icon::Share2, "Shares", Page::Shares),
                nav(Icon::Cloud, "Cloud sharing", Page::Cloud),
                nav(Icon::Server, "Remote mailboxes", Page::Remotes),
                nav(Icon::Settings, "Settings", Page::Settings),
            ]
            .spacing(1)
            .padding([4, 8]),
        )
        .style(style::scroll)
        .height(Fill),
        hairline(),
        footer,
    ]
    .height(Fill)
    .into()
}

fn confirm_bar(app: &App) -> Element<'_, Message> {
    let Some(confirm) = &app.confirm else {
        return space::vertical().height(0).into();
    };
    let (question, action) = match confirm {
        Confirm::ClearMailbox(name) => (format!("Delete all emails in “{name}”?"), "Delete"),
        Confirm::DeleteMailbox(name) => (
            format!("Delete mailbox “{name}” and all its emails?"),
            "Delete",
        ),
        Confirm::DeleteRemote(_) => (
            "Delete this remote mailbox and its emails?".to_string(),
            "Delete",
        ),
        Confirm::DeleteCloudShare(_) => (
            "Delete the uploaded file and the share?".to_string(),
            "Delete",
        ),
        Confirm::ForgetCloudShare(_, error) => (
            format!(
                "Couldn't delete the uploaded file ({error}). Remove the share from the list and leave the file in the bucket?"
            ),
            "Remove from list",
        ),
        Confirm::DeleteS3Config(_) => (
            "Remove this bucket? Shares already uploaded stay online.".to_string(),
            "Remove",
        ),
        Confirm::ResetVault => (
            "Reset the vault? This forgets the master password and deletes all synced buckets, here and in the cloud.".to_string(),
            "Reset",
        ),
    };
    container(
        row![
            icon(Icon::CircleAlert, 16.0, danger_c),
            tx(question, 13.0, fg).font(MEDIUM).width(Fill),
            btn(None, "Cancel", Kind::Secondary, Some(Message::ConfirmNo)),
            button(text(action).size(12.5).font(MEDIUM))
                .padding([6, 12])
                .style(|theme: &Theme, status| {
                    let t = style::tokens(theme);
                    let hovered = matches!(status, button::Status::Hovered);
                    button::Style {
                        background: Some(iced::Background::Color(if hovered {
                            t.danger.scale_alpha(0.85)
                        } else {
                            t.danger
                        })),
                        text_color: Color::WHITE,
                        border: iced::border::rounded(6),
                        ..Default::default()
                    }
                })
                .on_press(Message::ConfirmYes),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding(iced::Padding {
        top: 10.0 + TOP_INSET,
        right: 20.0,
        bottom: 10.0,
        left: 20.0,
    })
    .style(style::confirm_bar)
    .into()
}

// ---------------------------------------------------------------- list

fn email_list(app: &App) -> Element<'_, Message> {
    let title = app.mailbox.clone().unwrap_or_else(|| "All emails".into());
    let mut header = row![row![
        tx(title, 15.0, fg).font(SEMIBOLD),
        faint(app.emails.len().to_string()).size(12)
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .width(Fill)]
    .spacing(2)
    .align_y(Alignment::Center);
    if !app.emails.is_empty() {
        header = header.push(icon_btn(Icon::Share2, Message::ShareMailbox, false));
    }
    if let Some(name) = &app.mailbox {
        header = header
            .push(icon_btn(
                Icon::BrushCleaning,
                Message::Ask(Confirm::ClearMailbox(name.clone())),
                false,
            ))
            .push(icon_btn(
                Icon::Trash2,
                Message::Ask(Confirm::DeleteMailbox(name.clone())),
                true,
            ));
    }

    let search = stack![
        text_input("Search emails", &app.search)
            .id(SEARCH)
            .on_input(Message::Search)
            .padding(iced::Padding {
                top: 7.0,
                bottom: 7.0,
                left: 32.0,
                right: 10.0
            })
            .size(13)
            .style(style::input),
        container(icon(Icon::Search, 15.0, faint_c))
            .padding(iced::Padding {
                left: 10.0,
                ..Default::default()
            })
            .center_y(Fill),
    ]
    .height(32);

    let checked = app.checked.len();
    let mut head = column![header, search].spacing(10);
    if checked > 0 {
        head = head.push(
            row![
                checkbox(checked == app.emails.len())
                    .size(16)
                    .style(style::check)
                    .on_toggle(Message::CheckAll),
                one_line(muted(format!("{checked} selected"))),
                tip(
                    icon_btn(Icon::MailOpen, Message::MarkCheckedRead, false),
                    "Mark read"
                ),
                tip(
                    icon_btn(Icon::Share2, Message::ShareChecked, false),
                    "Share"
                ),
                tip(
                    icon_btn(Icon::FileDown, Message::ExportChecked, false),
                    "Export .html"
                ),
                btn(
                    Some(Icon::Trash2),
                    "Delete",
                    Kind::Danger,
                    Some(Message::DeleteChecked)
                ),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
    }

    let rows: Element<'_, Message> = if app.emails.is_empty() {
        if app.search.is_empty() {
            empty(
                Icon::Inbox,
                "No emails yet",
                "Point your app at the SMTP server and new mail shows up here instantly.",
                Some(
                    btn(
                        Some(Icon::Terminal),
                        "Setup & test email",
                        Kind::Secondary,
                        Some(Message::Page(Page::Setup)),
                    )
                    .into(),
                ),
            )
        } else {
            empty(
                Icon::Search,
                "No results",
                format!("Nothing matches “{}”.", app.search),
                None,
            )
        }
    } else {
        let count = app.emails.len();
        let range = visible_rows(count, app.list_scroll, list_height(app));
        let mut rows = Column::new()
            .padding(LIST_PAD)
            .push(space::vertical().height(range.start as f32 * ROW_STRIDE));
        for email in &app.emails[range.clone()] {
            rows = rows.push(
                container(email_row(app, email))
                    .height(ROW_STRIDE)
                    .align_y(Alignment::Start),
            );
        }
        rows = rows.push(space::vertical().height((count - range.end) as f32 * ROW_STRIDE));
        scrollable(rows)
            .id(LIST)
            .on_scroll(Message::ListScrolled)
            .style(style::scroll)
            .height(Fill)
            .into()
    };

    column![
        space::vertical().height(inset(app)),
        container(head).padding(iced::Padding {
            top: 12.0,
            right: 14.0,
            bottom: 10.0,
            left: 14.0
        }),
        hairline(),
        rows,
    ]
    .width(list_width(app))
    .height(Fill)
    .into()
}

fn email_row<'a>(app: &'a App, email: &'a EmailSummary) -> Element<'a, Message> {
    let active = app.selected.as_deref() == Some(email.id.as_str());
    let is_checked = app.checked.contains(&email.id);
    let (name, _) = split_from(&email.from);

    let mut chips = row![].spacing(4).align_y(Alignment::Center);
    if app.mailbox.is_none() {
        chips = chips.push(chip(ellipsize(&email.mailbox, 18), Tone::Neutral));
    }
    if email.has_attachments {
        chips = chips.push(chip_icon(Icon::Paperclip, "", Tone::Neutral));
    }
    if let Some(broken) = email.broken_links.filter(|n| *n > 0) {
        chips = chips.push(chip_icon(Icon::Link, format!("{broken} broken"), Tone::Bad));
    }
    if let Some(score) = email.spam_score.filter(|s| *s >= 2.5) {
        chips = chips.push(chip_icon(
            Icon::ShieldAlert,
            format!("{score:.1}"),
            spam_tone(score, 5.0),
        ));
    }

    // Fixed-height rows: one line each for sender, subject and preview (+ chips).
    let weight = if email.read { INTER } else { SEMIBOLD };
    let primary: Shade = if email.read { fg2 } else { fg };
    let subject = if email.subject.trim().is_empty() {
        "(no subject)".to_string()
    } else {
        ellipsize(&email.subject, 160)
    };
    let content = column![
        row![
            one_line(tx(ellipsize(&name, 120), 13.0, primary).font(weight)),
            faint(format_time(email.received_at)).wrapping(text::Wrapping::None)
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        one_line(tx(subject, 13.0, primary).font(weight)),
        row![
            one_line(tx(ellipsize(&email.preview, 160), 12.0, muted_c)),
            chips
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    ]
    .spacing(3);

    let select = || {
        container(
            checkbox(is_checked)
                .size(16)
                .style(style::check)
                .on_toggle(|value| Message::ToggleCheck(email.id.clone(), value)),
        )
        .width(32)
        .height(32)
        .center_x(32)
        .center_y(32)
    };
    let lead: Element<'_, Message> = if is_checked {
        select().into()
    } else {
        hover(avatar(&name, 32.0), select())
    };
    let unread_dot: Element<'_, Message> = if email.read {
        space::horizontal().width(5).into()
    } else {
        container(space::horizontal().width(5).height(5))
            .style(|theme: &Theme| style::dot(style::tokens(theme).accent)(theme))
            .into()
    };

    button(row![container(unread_dot).height(32).center_y(32), lead, content].spacing(8))
        .width(Fill)
        .height(ROW_H)
        .clip(true)
        .padding(iced::Padding {
            top: 10.0,
            right: 10.0,
            bottom: 8.0,
            left: 4.0,
        })
        .style(style::row(active))
        .on_press(Message::SelectEmail(email.id.clone()))
        .into()
}

// ---------------------------------------------------------------- detail

fn detail(app: &App) -> Element<'_, Message> {
    let Some(d) = &app.detail else {
        return column![
            space::vertical().height(inset(app)),
            if app.emails.is_empty() {
                empty(Icon::Mails, "Waiting for mail", "Captured emails will appear here.", None)
            } else {
                empty(
                    Icon::Mail,
                    "No email selected",
                    "Pick an email from the list. Use ↑ ↓ to move between emails and 1–9 to switch tabs.",
                    None,
                )
            }
        ]
        .into();
    };
    let s = &d.summary;
    let names = |list: &[scratchpad_core::Address]| {
        list.iter()
            .map(|a| a.name.clone().unwrap_or_else(|| a.email.clone()))
            .collect::<Vec<_>>()
    };
    let full = |list: &[scratchpad_core::Address]| {
        list.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };

    // Narrow windows drop the labels of the main actions.
    let compact = detail_width(app) < 640.0;
    let action = |glyph: Icon, label: &'static str, message: Message| -> Element<'_, Message> {
        if compact {
            tip(icon_btn(glyph, message, false), label)
        } else {
            btn(Some(glyph), label, Kind::Ghost, Some(message)).into()
        }
    };
    let toolbar = row![
        action(Icon::Share2, "Share", Message::ShareSelected),
        action(Icon::Send, "Forward", Message::ForwardStart),
        action(Icon::FileDown, "Export .html", Message::ExportSelected),
        action(
            if s.read { Icon::Mail } else { Icon::MailOpen },
            if s.read { "Mark unread" } else { "Mark read" },
            Message::ToggleRead
        ),
        space::horizontal(),
        faint(format_size(s.size)).wrapping(text::Wrapping::None),
        space::horizontal().width(6),
        tip(
            icon_btn(Icon::ExternalLink, Message::OpenPreviewInBrowser, false),
            "Open in browser"
        ),
        tip(
            icon_btn(Icon::Download, Message::SaveEml, false),
            "Save .eml"
        ),
        tip(
            icon_btn(Icon::RefreshCw, Message::Recheck, false),
            "Check links and spam again"
        ),
        container(container(space::vertical().width(1).height(18)).style(style::divider))
            .padding([0, 4]),
        tip(
            icon_btn(Icon::Trash2, Message::DeleteSelected, true),
            "Delete"
        ),
    ]
    .spacing(2)
    .align_y(Alignment::Center);

    let sender = d.from.first().cloned().unwrap_or(scratchpad_core::Address {
        name: None,
        email: d.envelope_from.clone(),
    });
    let sender_name = sender
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| sender.email.clone());
    let mut recipients = names(&d.to);
    recipients.extend(names(&d.cc));
    if recipients.is_empty() {
        recipients = d.envelope_to.clone();
    }
    let bcc: Vec<String> = d.bcc.iter().map(|a| a.email.clone()).collect();
    let mut to_line = format!("to {}", list_some(&recipients, if compact { 2 } else { 4 }));
    if !bcc.is_empty() {
        to_line.push_str(&format!(" · bcc {}", list_some(&bcc, 2)));
    }
    let date = d
        .date
        .as_deref()
        .and_then(format_rfc3339)
        .unwrap_or_else(|| format_full(s.received_at));

    let mut name_row = row![tx(ellipsize(&sender_name, 120), 13.0, fg).font(SEMIBOLD)]
        .spacing(6)
        .align_y(Alignment::Center);
    if sender.name.is_some() {
        name_row = name_row.push(muted(format!("<{}>", ellipsize(&sender.email, 64))));
    }
    let details_link = button(
        row![
            tx(
                if app.show_meta {
                    "Hide details"
                } else {
                    "Details"
                },
                12.0,
                muted_c
            ),
            icon(Icon::ChevronRight, 12.0, muted_c)
        ]
        .spacing(2)
        .align_y(Alignment::Center),
    )
    .padding(0)
    .style(style::btn(Kind::Link))
    .on_press(Message::ToggleMeta);
    let sender_row = row![
        avatar(&sender_name, 38.0),
        column![
            name_row.wrap(),
            muted(to_line).wrapping(text::Wrapping::WordOrGlyph)
        ]
        .spacing(2)
        .width(Fill),
        column![muted(date), details_link]
            .spacing(2)
            .align_x(Alignment::End),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let mut head = column![
        toolbar,
        space::vertical().height(4),
        tx(
            if s.subject.trim().is_empty() {
                "(no subject)".to_string()
            } else {
                ellipsize(&s.subject, SUBJECT_MAX)
            },
            20.0,
            fg
        )
        .font(BOLD)
        .wrapping(text::Wrapping::WordOrGlyph),
        space::vertical().height(6),
        sender_row
    ]
    .spacing(6);

    if app.show_meta {
        let mut grid = Column::new().spacing(4);
        // The full subject when the heading had to cut it.
        let long_subject = if s.subject.chars().count() > SUBJECT_MAX {
            s.subject.clone()
        } else {
            String::new()
        };
        for (label, value) in [
            ("Subject", long_subject),
            ("From", full(&d.from)),
            ("To", full(&d.to)),
            ("Cc", full(&d.cc)),
            ("Bcc", full(&d.bcc)),
            ("Reply-To", full(&d.reply_to)),
            ("Received", format_full(s.received_at)),
            ("Mailbox", s.mailbox.clone()),
            ("Message-ID", d.message_id.clone().unwrap_or_default()),
            (
                "Envelope",
                format!("{} → {}", d.envelope_from, d.envelope_to.join(", ")),
            ),
        ] {
            if !value.is_empty() {
                grid = grid.push(
                    row![
                        muted(label).width(90),
                        tx(ellipsize(&value, 2000), 12.5, fg2)
                            .wrapping(text::Wrapping::WordOrGlyph)
                    ]
                    .spacing(12),
                );
            }
        }
        head = head.push(
            container(grid)
                .padding([10, 14])
                .width(Fill)
                .style(style::code_block),
        );
    }

    if let Some(value) = &app.forward {
        head = head.push(
            container(
                row![
                    icon(Icon::Send, 15.0, muted_c),
                    text_input("Forward to (comma separated)", value)
                        .on_input(Message::ForwardInput)
                        .on_submit(Message::ForwardSend)
                        .padding([6, 10])
                        .size(13)
                        .style(style::field),
                    btn(
                        None,
                        if app.forwarding { "Sending…" } else { "Send" },
                        Kind::Primary,
                        (!app.forwarding).then_some(Message::ForwardSend)
                    ),
                    btn(None, "Cancel", Kind::Ghost, Some(Message::ForwardCancel)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([8, 12])
            .style(style::card),
        );
    }

    let broken = d.links.as_ref().map(|l| {
        l.iter()
            .filter(|l| matches!(l.status, LinkStatus::Broken { .. }))
            .count()
    });
    let tabs = Row::with_children(
        Tab::ALL
            .iter()
            .filter(|tab| **tab != Tab::Inspector || d.view_inspector.is_some())
            .map(|tab| {
                let active = app.tab == *tab;
                let badge: Option<Element<'_, Message>> = match tab {
                    Tab::Attachments if !d.attachments.is_empty() => {
                        Some(chip(d.attachments.len().to_string(), Tone::Neutral))
                    }
                    Tab::Links => match (broken, &d.links) {
                        (Some(b), _) if b > 0 => Some(chip(b.to_string(), Tone::Bad)),
                        (_, Some(links)) => Some(chip(links.len().to_string(), Tone::Neutral)),
                        _ => None,
                    },
                    Tab::Spam => d
                        .spam
                        .as_ref()
                        .map(|r| chip(format!("{:.1}", r.score), spam_tone(r.score, r.threshold))),
                    _ => None,
                };
                let mut label = row![text(tab.label()).size(13).font(MEDIUM)]
                    .spacing(6)
                    .align_y(Alignment::Center);
                if let Some(badge) = badge {
                    label = label.push(badge);
                }
                let underline = container(space::horizontal().height(2))
                    .height(2)
                    .width(Fill)
                    .style(move |theme: &Theme| {
                        if active {
                            style::dot(style::tokens(theme).fg)(theme)
                        } else {
                            container::Style::default()
                        }
                    });
                column![
                    button(label)
                        .padding([9, 0])
                        .style(style::tab(active))
                        .on_press(Message::Tab(*tab)),
                    underline
                ]
                .width(Length::Shrink)
                .into()
            }),
    )
    .spacing(if compact { 14 } else { 20 })
    .wrap()
    .vertical_spacing(0);

    column![
        space::vertical().height(inset(app)),
        // The header scrolls when it would leave too little room for the tab content
        // (short windows, long subjects, the details grid).
        container(column![
            container(
                scrollable(head)
                    .direction(scrollable::Direction::Vertical(
                        scrollable::Scrollbar::new()
                            .width(4)
                            .scroller_width(4)
                            .spacing(6)
                    ))
                    .style(style::scroll)
            )
            .max_height((app.window.height - inset(app) - 300.0).max(150.0)),
            space::vertical().height(10),
            tabs
        ])
        .padding(iced::Padding {
            top: 6.0,
            right: 24.0,
            bottom: 0.0,
            left: 24.0
        }),
        hairline(),
        container(tab_body(app)).height(Fill).width(Fill),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}

fn editor(app: &App) -> Element<'_, Message> {
    text_editor(&app.editor)
        .on_action(Message::Editor)
        .font(MONO)
        .size(12)
        .height(Fill)
        .padding([18, 24])
        .style(style::code_editor)
        .into()
}

fn table_row<'a>(
    left: Element<'a, Message>,
    right: Element<'a, Message>,
    left_width: Length,
) -> Element<'a, Message> {
    column![
        row![
            container(left).width(left_width),
            container(right).width(Fill)
        ]
        .spacing(16)
        .padding([9, 24]),
        hairline()
    ]
    .into()
}

fn tab_body(app: &App) -> Element<'_, Message> {
    let Some(d) = &app.detail else {
        return space::vertical().into();
    };
    match app.tab {
        Tab::Preview => {
            if d.html.is_none() && d.text.is_none() {
                return empty(
                    Icon::FileText,
                    "Empty email",
                    "This email has no body.",
                    None,
                );
            }
            preview::preview(app)
        }
        Tab::Text | Tab::Source | Tab::Raw => editor(app),
        Tab::Headers => {
            let mut rows = Column::new();
            let entries = [
                (
                    "SMTP MAIL FROM".to_string(),
                    if d.envelope_from.is_empty() {
                        "—".into()
                    } else {
                        d.envelope_from.clone()
                    },
                ),
                (
                    "SMTP RCPT TO".to_string(),
                    if d.envelope_to.is_empty() {
                        "—".into()
                    } else {
                        d.envelope_to.join(", ")
                    },
                ),
            ]
            .into_iter()
            .chain(d.headers.iter().map(|h| (h.name.clone(), h.value.clone())));
            for (name, value) in entries {
                rows = rows.push(table_row(
                    muted(name).into(),
                    tx(value, 12.0, fg2)
                        .font(MONO)
                        .wrapping(text::Wrapping::WordOrGlyph)
                        .into(),
                    Length::Fixed(if detail_width(app) < 640.0 {
                        130.0
                    } else {
                        190.0
                    }),
                ));
            }
            scrollable(rows).style(style::scroll).height(Fill).into()
        }
        Tab::Attachments => {
            if d.attachments.is_empty() {
                return empty(Icon::Paperclip, "No attachments", "", None);
            }
            let cards = d.attachments.iter().map(|a| {
                let mut title = row![tx(a.filename.clone(), 13.0, fg).font(SEMIBOLD)]
                    .spacing(8)
                    .align_y(Alignment::Center);
                if a.inline {
                    title = title.push(chip(
                        match &a.content_id {
                            Some(cid) => format!("inline · cid:{cid}"),
                            None => "inline".into(),
                        },
                        Tone::Neutral,
                    ));
                }
                let glyph = if a.content_type.starts_with("image/") {
                    Icon::Eye
                } else {
                    Icon::Paperclip
                };
                container(
                    row![
                        container(icon(glyph, 16.0, muted_c))
                            .width(36)
                            .height(36)
                            .center_x(36)
                            .center_y(36)
                            .style(style::tile),
                        column![
                            title,
                            muted(format!("{} · {}", a.content_type, format_size(a.size)))
                        ]
                        .spacing(2)
                        .width(Fill),
                        btn(
                            Some(Icon::ExternalLink),
                            "Open",
                            Kind::Secondary,
                            Some(Message::OpenAttachment(a.index))
                        ),
                        btn(
                            Some(Icon::Download),
                            "Save",
                            Kind::Secondary,
                            Some(Message::SaveAttachment(a.index))
                        ),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding([12, 16])
                .style(style::card)
                .into()
            });
            scrollable(Column::with_children(cards).spacing(10).padding([18, 24]))
                .style(style::scroll)
                .height(Fill)
                .into()
        }
        Tab::Links => {
            let Some(links) = &d.links else {
                let hint = if app.settings.link_checker {
                    "The link checker hasn't run for this email."
                } else {
                    "The link checker is turned off in Settings."
                };
                let action = btn(
                    Some(Icon::RefreshCw),
                    "Check links now",
                    Kind::Secondary,
                    Some(Message::Recheck),
                );
                return empty(Icon::Link, "Links not checked", hint, Some(action.into()));
            };
            if links.is_empty() {
                return empty(
                    Icon::Link,
                    "No links",
                    "This email doesn't contain any links.",
                    None,
                );
            }
            let rows = links.iter().map(|link| {
                let status = match &link.status {
                    LinkStatus::Pending => chip("Checking…", Tone::Neutral),
                    LinkStatus::Ok { status, method } => {
                        chip_icon(Icon::Check, format!("{status} {method}"), Tone::Ok)
                    }
                    LinkStatus::Skipped { reason } => chip(reason.clone(), Tone::Neutral),
                    LinkStatus::Broken { status, error } => chip(
                        match status {
                            Some(code) => format!("{code} · {error}"),
                            None => error.clone(),
                        },
                        Tone::Bad,
                    ),
                };
                column![
                    row![
                        container(status).width(Length::FillPortion(2)),
                        column![
                            button(
                                tx(ellipsize(&link.url, 500), 12.0, accent_c)
                                    .font(MONO)
                                    .wrapping(text::Wrapping::WordOrGlyph)
                            )
                            .padding(0)
                            .style(style::btn(Kind::Link))
                            .on_press(Message::OpenUrl(link.url.clone())),
                            muted(link.text.clone().unwrap_or_default()),
                        ]
                        .spacing(2)
                        .width(Length::FillPortion(5)),
                        icon_btn(Icon::Copy, Message::Copy(link.url.clone()), false),
                    ]
                    .spacing(16)
                    .align_y(Alignment::Center)
                    .padding([9, 24]),
                    hairline(),
                ]
                .into()
            });
            column![
                container(muted(
                    "Each link gets a HEAD request, then a GET request if that fails. Turn the checker off in Settings when you test one-time links."
                ))
                .padding([12, 24]),
                hairline(),
                scrollable(Column::with_children(rows)).style(style::scroll).height(Fill),
            ]
            .into()
        }
        Tab::Spam => {
            let Some(report) = &d.spam else {
                let hint = if app.settings.spam_check {
                    "The spam check hasn't run for this email."
                } else {
                    "Spam checks are turned off in Settings."
                };
                let action = btn(
                    Some(Icon::RefreshCw),
                    "Run spam check",
                    Kind::Secondary,
                    Some(Message::Recheck),
                );
                return empty(
                    Icon::ShieldAlert,
                    "No spam report yet",
                    hint,
                    Some(action.into()),
                );
            };
            let tone = spam_tone(report.score, report.threshold);
            let tone_shade: Shade = match tone {
                Tone::Bad => danger_c,
                Tone::Warn => |t| t.warn,
                _ => |t| t.ok,
            };
            let pct = ((report.score / (report.threshold * 2.0)).clamp(0.03, 1.0) * 1000.0) as u16;
            let fill = container(space::horizontal().height(6))
                .width(Length::FillPortion(pct.max(1)))
                .height(6)
                .style(move |theme: &Theme| style::dot(tone_shade(style::tokens(theme)))(theme));
            let meter = container(row![
                fill,
                space::horizontal().width(Length::FillPortion(1000u16.saturating_sub(pct).max(1)))
            ])
            .height(8)
            .padding(1)
            .style(style::code_block);
            let verdict = if report.is_spam {
                "Likely spam"
            } else if tone == Tone::Ok {
                "Looks good"
            } else {
                "Needs attention"
            };
            let mut content = Column::new().spacing(14).padding([18, 24]);
            if let Some(note) = &report.note {
                content = content.push(
                    container(
                        row![
                            icon(Icon::CircleAlert, 15.0, |t| t.warn),
                            text(note.clone()).size(12.5)
                        ]
                        .spacing(10),
                    )
                    .padding([10, 12])
                    .width(Fill)
                    .style(style::note),
                );
            }
            content = content.push(
                container(
                    row![
                        tx(format!("{:.1}", report.score), 34.0, tone_shade).font(BOLD),
                        column![
                            row![
                                chip(verdict, tone),
                                muted(format!(
                                    "threshold {:.1} · {}",
                                    report.threshold, report.engine
                                ))
                            ]
                            .spacing(8)
                            .align_y(Alignment::Center),
                            meter,
                        ]
                        .spacing(10)
                        .width(Fill),
                    ]
                    .spacing(18)
                    .align_y(Alignment::Center),
                )
                .padding(18)
                .style(style::card),
            );
            content = content.push(muted(
                "The score is only one signal. Receiving servers also weigh your sending server's reputation, SPF, DKIM and DMARC.",
            ));
            let mut rules = Column::new();
            for rule in &report.rules {
                rules = rules.push(column![
                    row![
                        tx(format!("{:.1}", rule.score), 12.0, fg2)
                            .font(MONO)
                            .width(48),
                        tx(rule.name.clone(), 12.0, fg2)
                            .font(MONO)
                            .width(Length::FillPortion(2)),
                        tx(rule.description.clone(), 12.5, fg2).width(Length::FillPortion(4)),
                    ]
                    .spacing(12)
                    .padding([9, 0]),
                    hairline()
                ]);
            }
            if report.rules.is_empty() {
                rules = rules.push(muted("No rules matched."));
            }
            scrollable(content.push(rules))
                .style(style::scroll)
                .height(Fill)
                .into()
        }
        Tab::Inspector => {
            let Some(inspector) = &d.view_inspector else {
                return empty(
                    Icon::Braces,
                    "No debug data",
                    "Install beyondcode/helo-laravel to see the view, data and source behind each mailable.",
                    None,
                );
            };
            let data = inspector
                .view_data_html
                .as_deref()
                .map(|html| scratchpad_core::parse::html_to_text(html, 100))
                .unwrap_or_else(|| "—".into());
            let section = |label: &'static str| muted(label).font(MEDIUM);
            let view_data_url = app
                .email_url(&format!("/api/emails/{}/view-data", d.summary.id))
                .map(Message::OpenUrl);
            column![
                column![
                    section("View file"),
                    container(
                        tx(
                            inspector.view_file.clone().unwrap_or_else(|| "—".into()),
                            12.5,
                            fg
                        )
                        .font(MONO)
                    )
                    .padding([10, 14])
                    .width(Fill)
                    .style(style::code_block),
                    row![
                        section("View data").width(Fill),
                        btn(
                            Some(Icon::ExternalLink),
                            "Open in browser",
                            Kind::Ghost,
                            view_data_url
                        )
                    ]
                    .align_y(Alignment::Center),
                    container(
                        scrollable(tx(data, 12.0, fg2).font(MONO))
                            .style(style::scroll)
                            .height(140)
                    )
                    .padding([10, 14])
                    .width(Fill)
                    .style(style::code_block),
                    section("View source"),
                ]
                .spacing(8)
                .padding(iced::Padding {
                    top: 18.0,
                    right: 24.0,
                    bottom: 4.0,
                    left: 24.0
                }),
                editor(app),
            ]
            .into()
        }
    }
}

// ---------------------------------------------------------------- pages

pub(super) fn tile_icon<'a>(glyph: Icon) -> Element<'a, Message> {
    container(icon(glyph, 16.0, muted_c))
        .width(36)
        .height(36)
        .center_x(36)
        .center_y(36)
        .style(style::tile)
        .into()
}

fn setup(app: &App) -> Element<'_, Message> {
    let address = &app.status.smtp_address;
    let (host, port) = address
        .rsplit_once(':')
        .unwrap_or((address.as_str(), "2525"));
    let host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    let snippets = [
        ("Laravel", ".env", format!("MAIL_MAILER=smtp\nMAIL_HOST={host}\nMAIL_PORT={port}\nMAIL_USERNAME=Inbox-Name\nMAIL_PASSWORD=null\nMAIL_ENCRYPTION=null")),
        ("Symfony", ".env", format!("MAILER_DSN=smtp://Mailbox-Name:@{host}:{port}")),
        ("Nodemailer", "JavaScript", format!("nodemailer.createTransport({{\n  host: \"{host}\",\n  port: {port},\n  auth: {{ user: \"Mailbox-Name\", pass: \"\" }},\n}});")),
        ("Django", "settings.py", format!("EMAIL_HOST = \"{host}\"\nEMAIL_PORT = {port}\nEMAIL_HOST_USER = \"Mailbox-Name\"\nEMAIL_HOST_PASSWORD = \"\"")),
        ("Ruby on Rails", "config/environments", format!("config.action_mailer.delivery_method = :smtp\nconfig.action_mailer.smtp_settings = {{\n  user_name: 'Mailbox-Name', password: '',\n  address: '{host}', port: {port},\n  authentication: :plain\n}}")),
        ("WordPress", "functions.php", format!("add_action('phpmailer_init', function ($m) {{\n  $m->isSMTP();\n  $m->Host = '{host}';\n  $m->Port = {port};\n  $m->SMTPAuth = true;\n  $m->Username = 'Mailbox-Name';\n  $m->Password = '';\n}});")),
        ("Go", "net/smtp", format!("auth := smtp.PlainAuth(\"\", \"Mailbox-Name\", \"\", \"{host}\")\nsmtp.SendMail(\"{host}:{port}\", auth, from, to, msg)")),
        ("Docker & VMs", "hosts", "Docker:  host.docker.internal\nVagrant: 10.0.2.2\n\nSet the SMTP host to 0.0.0.0 in Settings\nso Scratchpad accepts these connections.".to_string()),
    ];
    let hero = container(
        row![
            tile_icon(Icon::Mail),
            column![
                muted("SMTP server"),
                tx(format!("{host}:{port}"), 16.0, fg).font(Font {
                    weight: Weight::Semibold,
                    ..MONO
                })
            ]
            .spacing(2)
            .width(Fill),
            text_input("Mailbox", &app.test_mailbox)
                .on_input(Message::TestMailbox)
                .width(140)
                .padding([7, 10])
                .size(13)
                .style(style::field),
            btn(
                Some(Icon::Send),
                if app.sending_test {
                    "Sending…"
                } else {
                    "Send test email"
                },
                Kind::Primary,
                (!app.sending_test).then_some(Message::SendTestEmail)
            ),
        ]
        .spacing(14)
        .align_y(Alignment::Center),
    )
    .padding(18)
    .style(style::hero);

    // Code doesn't wrap (it scrolls sideways in narrow windows); both cards of a
    // row are as tall as the longer snippet.
    let snippet = |(title, file, code): (&'static str, &'static str, String),
                   lines: usize|
     -> Element<'_, Message> {
        container(column![
            row![
                row![
                    tx(title, 12.5, fg2).font(MEDIUM),
                    faint(format!("· {file}")).size(12)
                ]
                .spacing(6)
                .width(Fill),
                button(icon(Icon::Copy, 14.0, muted_c))
                    .padding(4)
                    .style(style::btn(Kind::Icon))
                    .on_press(Message::Copy(code.clone())),
            ]
            .align_y(Alignment::Center)
            .padding(iced::Padding {
                top: 6.0,
                right: 6.0,
                bottom: 6.0,
                left: 14.0
            }),
            hairline(),
            container(
                scrollable(
                    container(
                        tx(code, 11.5, fg2)
                            .font(MONO)
                            .line_height(1.6)
                            .wrapping(text::Wrapping::None)
                    )
                    .padding([12, 14])
                )
                .direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::new().width(4).scroller_width(4)
                ))
                .style(style::scroll)
                .width(Fill)
            )
            .width(Fill)
            .height(lines as f32 * 11.5 * 1.6 + 32.0)
            .style(style::code_block),
        ])
        .width(Fill)
        .style(style::card)
        .into()
    };
    let mut grid = Column::new().spacing(12);
    let mut iter = snippets.into_iter();
    let line_count = |s: &(&str, &str, String)| s.2.lines().count();
    while let Some(first) = iter.next() {
        let second = iter.next();
        let lines = line_count(&first).max(second.as_ref().map_or(0, line_count));
        let mut pair = Row::new().spacing(12).push(snippet(first, lines));
        pair = match second {
            Some(second) => pair.push(snippet(second, lines)),
            None => pair.push(space::horizontal()),
        };
        grid = grid.push(pair);
    }

    column![
        page_head(
            "Setup",
            "Point any app that sends mail over SMTP at Scratchpad. The SMTP username picks the mailbox and any password works. Mail sent without credentials lands in “Default”."
        ),
        hero,
        grid,
        tx("Laravel view inspector", 13.0, fg).font(SEMIBOLD),
        muted("Run `composer require --dev beyondcode/helo-laravel` to see the view file, view data and Blade source of every mailable in the Inspector tab."),
    ]
    .spacing(20)
    .into()
}

fn group_head<'a>(title: &'a str, lead: &'a str) -> Element<'a, Message> {
    column![tx(title, 14.0, fg).font(SEMIBOLD), muted(lead)]
        .spacing(2)
        .padding(iced::Padding {
            top: 8.0,
            ..Default::default()
        })
        .into()
}

fn shares(app: &App) -> Element<'_, Message> {
    let mut content =
        column![page_head("Shares", "Anyone with a share link can view the shared emails, read-only. Delete a share to revoke access.")]
            .spacing(12);
    if app.shares.is_empty() && app.cloud.shares.is_empty() {
        let action = app.cloud.configs.is_empty().then(|| {
            btn(
                Some(Icon::Cloud),
                "Set up cloud sharing",
                Kind::Secondary,
                Some(Message::Page(Page::Cloud)),
            )
            .into()
        });
        content = content.push(
            container(empty(
                Icon::Share2,
                "Nothing shared yet",
                "Use Share on an email or a mailbox to upload it to your bucket or create a link served by this app.",
                action,
            ))
            .height(320),
        );
        return content.into();
    }

    content = content.push(group_head(
        "Cloud shares",
        "Uploaded to your S3 buckets as a single .html file. They keep working when this app is offline.",
    ));
    if app.cloud.shares.is_empty() {
        let mut hint = row![
            icon(Icon::CloudUpload, 15.0, muted_c),
            muted(if app.cloud.configs.is_empty() {
                "Add a bucket to upload shares that work without this app."
            } else {
                "Choose a bucket in the Share dialog to upload emails."
            })
            .width(Fill)
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if app.cloud.configs.is_empty() {
            hint = hint.push(btn(
                None,
                "Set up cloud sharing",
                Kind::Ghost,
                Some(Message::Page(Page::Cloud)),
            ));
        }
        content = content.push(container(hint).padding([10, 16]).style(style::card));
    }
    for card in cloud::cloud_share_cards(app) {
        content = content.push(card);
    }

    content = content.push(group_head(
        "Server links",
        "Served live by this Scratchpad through the web interface.",
    ));
    if app.shares.is_empty() {
        content = content.push(
            container(muted("No server links."))
                .padding([10, 16])
                .width(Fill)
                .style(style::card),
        );
    }
    for share in &app.shares {
        let url = app.scratchpad.share_url(&share.token);
        let mut actions = row![].spacing(2);
        if let Some(url) = &url {
            actions = actions
                .push(icon_btn(Icon::Copy, Message::Copy(url.clone()), false))
                .push(icon_btn(
                    Icon::ExternalLink,
                    Message::OpenUrl(url.clone()),
                    false,
                ));
        }
        actions = actions.push(icon_btn(
            Icon::Trash2,
            Message::DeleteShare(share.token.clone()),
            true,
        ));
        let count = share.email_ids.len();
        let what = match &share.mailbox {
            Some(mailbox) => format!("Whole inbox “{mailbox}”, live"),
            None => format!("{count} email{}", if count == 1 { "" } else { "s" }),
        };
        let summary = format!(
            "{what} · {} · {}",
            format_time(share.created_at),
            url.unwrap_or_else(|| "enable the web interface to get a link".into())
        );
        content = content.push(
            container(
                row![
                    tile_icon(Icon::Link),
                    column![
                        tx(share.title.clone(), 13.0, fg).font(SEMIBOLD),
                        muted(summary)
                    ]
                    .spacing(2)
                    .width(Fill),
                    actions,
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding([12, 16])
            .style(style::card),
        );
    }
    content.into()
}

/// A settings row: label (+ help) on the left, control on the right.
pub(super) fn setting<'a>(
    label: &'a str,
    help: Option<&'a str>,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut left = column![tx(label, 13.0, fg).font(MEDIUM)]
        .spacing(2)
        .width(Length::FillPortion(5));
    if let Some(help) = help {
        left = left.push(muted(help));
    }
    row![
        left,
        container(control)
            .width(Length::FillPortion(4))
            .align_x(Alignment::End)
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .padding([12, 18])
    .into()
}

pub(super) fn section<'a>(
    title: &'a str,
    lead: Option<&'a str>,
    rows: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut head = column![tx(title, 13.5, fg).font(SEMIBOLD)].spacing(2);
    if let Some(lead) = lead {
        head = head.push(muted(lead));
    }
    let mut body = column![container(head).padding([14, 18]).width(Fill)];
    for row in rows {
        body = body.push(hairline()).push(row);
    }
    container(body).width(Fill).style(style::card).into()
}

fn remotes(app: &App) -> Element<'_, Message> {
    let mut content = column![page_head(
        "Remote mailboxes",
        "Scratchpad polls each remote mailbox and shows new mail like a local one. Pull from an IMAP account, or from another Scratchpad running headless on your server."
    )]
    .spacing(12);

    if let Some(form) = &app.remote_form {
        let input = |placeholder: &'static str, value: &str, field: RemoteField| {
            text_input(placeholder, value)
                .on_input(move |v| Message::Remote(RemoteMsg::Text(field, v)))
                .padding([7, 10])
                .size(13)
                .style(style::field)
        };
        let switch = |value: bool, f: fn(bool) -> RemoteMsg| {
            toggler(value)
                .size(18)
                .style(style::switch)
                .on_toggle(move |v| Message::Remote(f(v)))
        };
        let kind = pick_list(
            [RemoteKind::Imap, RemoteKind::Scratchpad],
            Some(form.kind),
            |kind| Message::Remote(RemoteMsg::Kind(kind)),
        )
        .text_size(13)
        .padding([7, 10])
        .style(style::select);
        let mut rows = vec![
            setting(
                "Name",
                None,
                input("Staging", &form.name, RemoteField::Name),
            ),
            setting("Source", None, kind),
            setting("Enabled", None, switch(form.enabled, RemoteMsg::Enabled)),
        ];
        match form.kind {
            RemoteKind::Imap => rows.extend([
                setting(
                    "IMAP host",
                    None,
                    input("imap.example.com", &form.host, RemoteField::Host),
                ),
                setting("Port", None, input("993", &form.port, RemoteField::Port)),
                setting("TLS", None, switch(form.tls, RemoteMsg::Tls)),
                setting(
                    "Username",
                    None,
                    input("user@example.com", &form.username, RemoteField::Username),
                ),
                setting(
                    "Password",
                    None,
                    input("", &form.password, RemoteField::Password).secure(true),
                ),
                setting(
                    "Folder",
                    None,
                    input("INBOX", &form.folder, RemoteField::Folder),
                ),
                setting(
                    "Delete after fetch",
                    Some("Remove messages from the server once imported."),
                    switch(form.delete_after_fetch, RemoteMsg::DeleteAfterFetch),
                ),
            ]),
            RemoteKind::Scratchpad => rows.extend([
                setting(
                    "Server URL",
                    None,
                    input(
                        "https://scratchpad.staging.example.com",
                        &form.base_url,
                        RemoteField::BaseUrl,
                    ),
                ),
                setting(
                    "Mailbox",
                    None,
                    input("Default", &form.mailbox, RemoteField::Mailbox),
                ),
                setting(
                    "Username",
                    Some("Only if the server uses basic auth."),
                    input("optional", &form.username, RemoteField::Username),
                ),
                setting(
                    "Password",
                    None,
                    input("optional", &form.password, RemoteField::Password).secure(true),
                ),
            ]),
        }
        rows.push(
            container(
                row![
                    btn(None, "Cancel", Kind::Secondary, Some(Message::CancelRemote)),
                    btn(
                        Some(Icon::Check),
                        "Save",
                        Kind::Primary,
                        Some(Message::SaveRemote)
                    )
                ]
                .spacing(8),
            )
            .width(Fill)
            .align_x(Alignment::End)
            .padding([12, 18])
            .into(),
        );
        content = content.push(section(
            if form.id.is_empty() {
                "Add remote mailbox"
            } else {
                "Edit remote mailbox"
            },
            None,
            rows,
        ));
    }

    if app.remotes.is_empty() && app.remote_form.is_none() {
        content = content.push(
            container(empty(
                Icon::CloudDownload,
                "No remote mailboxes",
                "Test live and staging apps from here.",
                None,
            ))
            .height(280),
        );
    }
    for remote in &app.remotes {
        let (kind, glyph, detail) = match &remote.source {
            RemoteSource::Imap {
                host,
                port,
                username,
                folder,
                ..
            } => (
                "IMAP",
                Icon::AtSign,
                format!("{username} @ {host}:{port}/{folder}"),
            ),
            RemoteSource::Scratchpad {
                base_url, mailbox, ..
            } => (
                "Scratchpad",
                Icon::Server,
                format!("{base_url} → {mailbox}"),
            ),
        };
        let mut title = row![
            tx(remote.name.clone(), 13.0, fg).font(SEMIBOLD),
            chip(kind, Tone::Neutral)
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if !remote.enabled {
            title = title.push(chip("Paused", Tone::Warn));
        }
        let checked = match remote.last_poll_at {
            Some(ts) => format!("checked {}", format_time(ts)),
            None => "not checked yet".into(),
        };
        let mut info = column![title, muted(format!("{detail} · {checked}"))]
            .spacing(2)
            .width(Fill);
        if let Some(error) = &remote.last_error {
            info = info.push(tx(error.clone(), 12.0, danger_c));
        }
        content = content.push(
            container(
                row![
                    tile_icon(glyph),
                    info,
                    btn(
                        None,
                        "Edit",
                        Kind::Secondary,
                        Some(Message::EditRemote(remote.id.clone()))
                    ),
                    icon_btn(
                        Icon::Trash2,
                        Message::Ask(Confirm::DeleteRemote(remote.id.clone())),
                        true
                    ),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding([12, 16])
            .style(style::card),
        );
    }
    content = content.push(
        row![
            btn(
                Some(Icon::Plus),
                "Add remote mailbox",
                Kind::Primary,
                Some(Message::NewRemote)
            ),
            btn(
                Some(Icon::RefreshCw),
                "Check now",
                Kind::Secondary,
                Some(Message::PollRemotes)
            ),
        ]
        .spacing(8),
    );
    content.into()
}

fn settings(app: &App) -> Element<'_, Message> {
    let form = &app.settings_form;
    let input = |field: Field| {
        text_input("", form.text(field))
            .on_input(move |v| Message::Settings(SettingsMsg::Text(field, v)))
            .padding([7, 10])
            .size(13)
            .style(style::field)
    };
    let secret = |field: Field| input(field).secure(true);
    let switch = |t: Toggle| {
        toggler(form.toggle(t))
            .size(18)
            .style(style::switch)
            .on_toggle(move |v| Message::Settings(SettingsMsg::Toggle(t, v)))
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Enc(ForwardEncryption);
    impl std::fmt::Display for Enc {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(match self.0 {
                ForwardEncryption::None => "None",
                ForwardEncryption::StartTls => "STARTTLS",
                ForwardEncryption::Tls => "TLS",
            })
        }
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct ThemeChoice(ThemePreference);
    impl std::fmt::Display for ThemeChoice {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(match self.0 {
                ThemePreference::System => "System",
                ThemePreference::Light => "Light",
                ThemePreference::Dark => "Dark",
            })
        }
    }
    let encryption = pick_list(
        [
            Enc(ForwardEncryption::None),
            Enc(ForwardEncryption::StartTls),
            Enc(ForwardEncryption::Tls),
        ],
        Some(Enc(form.draft.forward.encryption)),
        |e| Message::Settings(SettingsMsg::Encryption(e.0)),
    )
    .text_size(13)
    .padding([7, 10])
    .style(style::select);
    let theme = pick_list(
        [
            ThemeChoice(ThemePreference::System),
            ThemeChoice(ThemePreference::Light),
            ThemeChoice(ThemePreference::Dark),
        ],
        Some(ThemeChoice(form.draft.theme)),
        |t| Message::Settings(SettingsMsg::Theme(t.0)),
    )
    .text_size(13)
    .padding([7, 10])
    .style(style::select);

    column![
        page_head("Settings", format!("Saved to {}", app.status.data_dir)),
        section(
            "SMTP server",
            Some("Where your apps deliver mail."),
            vec![
                setting(
                    "Host",
                    Some("0.0.0.0 accepts mail from Docker, VMs and other machines."),
                    input(Field::SmtpHost)
                ),
                setting("Port", None, input(Field::SmtpPort)),
                setting(
                    "Advertise STARTTLS",
                    Some("Self-signed certificate. Only for clients that require TLS."),
                    switch(Toggle::StartTls)
                ),
                setting(
                    "Require authentication",
                    Some("When off, mail without credentials goes to “Default”."),
                    switch(Toggle::RequireAuth)
                ),
                setting("Max message size", Some("In bytes."), input(Field::MaxSize)),
            ]
        ),
        section(
            "Checks",
            Some("Run on every incoming email."),
            vec![
                setting(
                    "Link checker",
                    Some("Turn off when testing one-time links like email verification."),
                    switch(Toggle::LinkChecker)
                ),
                setting("Link timeout (s)", None, input(Field::LinkTimeout)),
                setting(
                    "Spam report",
                    Some("SpamAssassin's spamd when reachable, otherwise built-in rules."),
                    switch(Toggle::SpamCheck)
                ),
                setting("spamd host", None, input(Field::SpamdHost)),
                setting("spamd port", None, input(Field::SpamdPort)),
            ]
        ),
        section(
            "Forwarding",
            Some("SMTP server used to forward captured emails to real inboxes."),
            vec![
                setting(
                    "Host",
                    Some("e.g. smtp.postmarkapp.com"),
                    input(Field::FwdHost)
                ),
                setting("Port", None, input(Field::FwdPort)),
                setting("Encryption", None, encryption),
                setting("Username", None, input(Field::FwdUser)),
                setting("Password", None, secret(Field::FwdPass)),
                setting(
                    "From override",
                    Some("Envelope sender. Empty keeps the original."),
                    input(Field::FwdFrom)
                ),
            ]
        ),
        section(
            "Web interface",
            Some("Host and port changes apply after a restart."),
            vec![
                setting("Host", None, input(Field::WebHost)),
                setting("Port", None, input(Field::WebPort)),
                setting(
                    "Username",
                    Some("Set a username or password to require basic auth."),
                    input(Field::WebUser)
                ),
                setting("Password", None, secret(Field::WebPass)),
                setting(
                    "Public URL",
                    Some("Base URL for share links, e.g. https://scratchpad.example.com"),
                    input(Field::PublicUrl)
                ),
            ]
        ),
        section(
            "General",
            None,
            vec![
                setting("Desktop notifications", None, switch(Toggle::Notifications)),
                setting("Theme", None, theme),
                setting("Remote poll interval (s)", None, input(Field::PollSecs)),
                setting(
                    "Delete emails after (days)",
                    Some("0 keeps emails forever."),
                    input(Field::Retention)
                ),
            ]
        ),
        container(btn(
            Some(Icon::Check),
            "Save changes",
            Kind::Primary,
            Some(Message::SaveSettings)
        ))
        .width(Fill)
        .align_x(Alignment::End),
    ]
    .spacing(16)
    .into()
}
