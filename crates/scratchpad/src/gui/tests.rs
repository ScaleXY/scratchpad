//! State and interaction tests for the desktop GUI. They run headless: `App` is driven
//! through `update`, and `iced_test::Simulator` renders with wgpu or, without a GPU,
//! tiny-skia.

use std::sync::Arc;

use iced::keyboard::key::{Code, NativeCode, Physical};
use iced::keyboard::{self, key::Named, Key, Location, Modifiers};
use iced_test::Simulator;
use scratchpad_core::{Event, Scratchpad, Settings};

use super::cloud::CloudMsg;
use super::forms::{Field, S3Msg, SettingsMsg};
use super::preview::{self, Device};
use super::{view, App, Confirm, Message, Page, Tab};

struct Fixture {
    runtime: tokio::runtime::Runtime,
    scratchpad: Arc<Scratchpad>,
    app: App,
}

impl Fixture {
    fn new(count: usize) -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let scratchpad = {
            let _guard = runtime.enter();
            let scratchpad = Scratchpad::in_memory(Settings {
                link_checker: false,
                spam_check: false,
                notifications: false,
                ..Settings::default()
            })
            .unwrap();
            for i in 0..count {
                scratchpad
                    .ingest_local(
                        email(i),
                        if i % 2 == 0 { "Acme" } else { "Billing" },
                        "app@example.com".into(),
                        vec!["dev@example.com".into()],
                    )
                    .unwrap();
            }
            scratchpad
        };
        let (app, _) = App::new(scratchpad.clone(), runtime.handle().clone());
        Self {
            runtime,
            scratchpad,
            app,
        }
    }

    fn send(&mut self, message: Message) {
        let _guard = self.runtime.enter();
        let _ = self.app.update(message);
    }

    fn key(&mut self, event: keyboard::Event) {
        self.send(Message::Keyboard(event));
    }

    fn ids(&self) -> Vec<String> {
        self.app.emails.iter().map(|e| e.id.clone()).collect()
    }

    /// Clicks `text` in a simulator showing the current view and applies the messages.
    fn click(&mut self, text: &str) {
        let messages = {
            let mut ui = simulator(&self.app);
            ui.click(text)
                .unwrap_or_else(|e| panic!("click {text:?}: {e}"));
            ui.into_messages().collect::<Vec<_>>()
        };
        assert!(!messages.is_empty(), "clicking {text:?} did nothing");
        for message in messages {
            self.send(message);
        }
    }
}

fn email(i: usize) -> Vec<u8> {
    format!(
        "From: Sender {i} <sender{i}@example.com>\r\nTo: dev@example.com\r\nSubject: Message {i}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain\r\n\r\nBody of message {i}.\r\n"
    )
    .into_bytes()
}

fn simulator(app: &App) -> Simulator<'_, Message> {
    Simulator::with_size(
        view::app_settings(),
        (app.window.width, app.window.height),
        app.view(),
    )
}

fn press(key: Key) -> keyboard::Event {
    press_with(key, Physical::Unidentified(NativeCode::Unidentified), false)
}

fn press_with(key: Key, physical_key: Physical, repeat: bool) -> keyboard::Event {
    keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key,
        location: Location::Standard,
        modifiers: Modifiers::empty(),
        text: None,
        repeat,
    }
}

fn char_key(c: &str) -> Key {
    Key::Character(c.into())
}

// ---------------------------------------------------------------- pure helpers

#[test]
fn preview_scaling() {
    let (screens, scale) = preview::layout(Device::Fit, false, 800.0);
    assert!(screens.is_empty());
    assert_eq!(scale, 1.0);

    // Never scaled up.
    let (screens, scale) = preview::layout(Device::Mobile, false, 1200.0);
    assert_eq!(screens, vec![(Device::Mobile, 390.0, 844.0)]);
    assert_eq!(scale, 1.0);

    // Scaled down to the pane minus padding.
    let (_, scale) = preview::layout(Device::Desktop, false, 700.0);
    assert!((scale - (700.0 - 48.0) / 1280.0).abs() < 1e-6);

    // Tablets and phones rotate, desktops don't.
    let (screens, _) = preview::layout(Device::Tablet, true, 700.0);
    assert_eq!(screens, vec![(Device::Tablet, 1024.0, 768.0)]);
    let (screens, _) = preview::layout(Device::Laptop, true, 700.0);
    assert_eq!(screens, vec![(Device::Laptop, 1024.0, 640.0)]);

    // "All" fits three screens plus gaps.
    let (screens, scale) = preview::layout(Device::All, false, 1000.0);
    assert_eq!(screens.len(), 3);
    let total = 390.0 + 768.0 + 1280.0 + 2.0 * 24.0;
    assert!((scale - (1000.0 - 48.0) / total).abs() < 1e-6);

    // Tiny panes keep a positive scale.
    let (_, scale) = preview::layout(Device::All, false, 10.0);
    assert!(scale > 0.0);
}

#[test]
fn list_virtualization_math() {
    // Short lists build every row.
    assert_eq!(view::visible_rows(5, 0.0, 600.0), 0..5);
    // Long lists build the visible rows plus some overscan.
    let rows = view::visible_rows(10_000, 0.0, 600.0);
    assert_eq!(rows.start, 0);
    assert!(rows.end < 30);
    let rows = view::visible_rows(10_000, 5000.0 * view::ROW_STRIDE, 600.0);
    assert!(rows.contains(&5000) && rows.len() < 40);
    // A stale offset after the list shrank still shows the last rows.
    let rows = view::visible_rows(3, 1_000_000.0, 600.0);
    assert_eq!(rows, 0..3);
    assert!(view::visible_rows(0, 0.0, 600.0).is_empty());

    // Rows already on screen don't scroll; others scroll just enough.
    assert_eq!(view::reveal_offset(0, 0.0, 600.0), None);
    let y = view::reveal_offset(20, 0.0, 600.0).unwrap();
    assert!(y > 0.0 && view::reveal_offset(20, y, 600.0).is_none());
    assert_eq!(view::reveal_offset(0, 500.0, 600.0), Some(0.0));
}

#[test]
fn text_helpers() {
    assert_eq!(view::ellipsize("short", 10), "short");
    assert_eq!(view::ellipsize("ünïcödé text", 5), "ünïcö…");
    assert_eq!(super::truncate_preview("abc".into()), "abc");
    let long = "x".repeat(super::PREVIEW_MAX + 10);
    let cut = super::truncate_preview(long);
    assert!(cut.starts_with(&"x".repeat(super::PREVIEW_MAX)));
    assert!(cut.contains("Open in browser"));
}

#[test]
fn file_names() {
    use super::{is_executable, sanitize};
    assert_eq!(sanitize("../../etc/passwd"), "_.._etc_passwd");
    assert_eq!(sanitize(".."), "email");
    assert_eq!(sanitize(".hidden"), "hidden");
    assert_eq!(sanitize("report. "), "report");
    assert_eq!(sanitize("CON.txt"), "_CON.txt");
    assert_eq!(sanitize("com1"), "_com1");
    assert_eq!(sanitize("Rechnung März.pdf"), "Rechnung März.pdf");
    let long = format!("{}.pdf", "a".repeat(200));
    let cut = sanitize(&long);
    assert!(cut.ends_with(".pdf") && cut.chars().count() <= 80);

    assert!(is_executable("setup.EXE"));
    assert!(is_executable("run.command"));
    assert!(is_executable("invoice.pdf.js"));
    assert!(!is_executable("invoice.pdf"));
    assert!(!is_executable("README"));
}

#[test]
fn only_web_links_open() {
    assert!(super::open_url("file:///etc/passwd").is_err());
    assert!(super::open_url("javascript:alert(1)").is_err());
    assert!(super::open_url("ms-msdt:/id").is_err());
    assert!(super::open_url("no scheme").is_err());
}

#[test]
fn scratch_files_are_private_and_unique() {
    let a = super::scratch_file("../a.html").unwrap();
    let b = super::scratch_file("a.html").unwrap();
    assert_ne!(a, b);
    assert_eq!(a.file_name().unwrap(), "_a.html");
    let dir = super::scratch_dir().unwrap();
    assert!(a.starts_with(&dir) && b.starts_with(&dir));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}

#[test]
fn shortcut_keys_ignore_layout() {
    let unidentified = Physical::Unidentified(NativeCode::Unidentified);
    assert_eq!(
        super::shortcut_char(&char_key("j"), unidentified),
        Some('j')
    );
    // Cyrillic layout: the J key types "о".
    assert_eq!(
        super::shortcut_char(&char_key("о"), Physical::Code(Code::KeyJ)),
        Some('j')
    );
    // AZERTY: the 2 key types "é" without Shift.
    assert_eq!(
        super::shortcut_char(&char_key("é"), Physical::Code(Code::Digit2)),
        Some('2')
    );
}

// ---------------------------------------------------------------- App state

#[test]
fn select_marks_read_and_switches_tabs() {
    let mut f = Fixture::new(3);
    let id = f.ids()[0].clone();
    assert!(!f.app.emails[0].read);
    f.send(Message::SelectEmail(id.clone()));
    assert_eq!(f.app.selected.as_deref(), Some(id.as_str()));
    assert!(
        f.app.emails[0].read,
        "the list updates without waiting for events"
    );
    assert!(f.app.detail.as_ref().unwrap().summary.read);
    assert!(f.app.preview_text.contains("Body of message"));

    f.send(Message::Tab(Tab::Raw));
    assert!(f.app.editor.text().contains("Subject: Message"));
    f.send(Message::Tab(Tab::Text));
    assert!(f.app.editor.text().contains("Body of message"));

    f.send(Message::ToggleRead);
    assert!(!f.app.detail.as_ref().unwrap().summary.read);
    assert!(!f.app.emails[0].read);
}

#[test]
fn keyboard_navigation_and_delete() {
    let mut f = Fixture::new(4);
    let ids = f.ids();
    f.send(Message::Navigate(true));
    assert_eq!(f.app.selected.as_ref(), Some(&ids[0]));
    f.key(press(char_key("j")));
    assert_eq!(f.app.selected.as_ref(), Some(&ids[1]));
    f.key(press(char_key("k")));
    f.key(press(char_key("k"))); // already at the top
    assert_eq!(f.app.selected.as_ref(), Some(&ids[0]));

    // 1-9 switch tabs (no Inspector tab for these emails).
    f.key(press(char_key("4")));
    assert_eq!(f.app.tab, Tab::Raw);
    f.key(press(char_key("9")));
    assert_eq!(f.app.tab, Tab::Raw);

    // Held-down Delete doesn't delete one email after another.
    f.key(press_with(
        Key::Named(Named::Delete),
        Physical::Code(Code::Delete),
        true,
    ));
    assert_eq!(f.app.emails.len(), 4);
    f.key(press(Key::Named(Named::Backspace)));
    assert_eq!(f.app.emails.len(), 3);
    assert_eq!(
        f.app.selected.as_ref(),
        Some(&ids[1]),
        "the next email is selected"
    );
}

#[test]
fn keyboard_shortcuts_are_off_in_dialogs_and_forms() {
    let mut f = Fixture::new(3);
    let ids = f.ids();
    f.send(Message::SelectEmail(ids[0].clone()));

    // Modifier shortcuts belong to the OS / widgets.
    let mut ctrl = press(char_key("j"));
    if let keyboard::Event::KeyPressed { modifiers, .. } = &mut ctrl {
        *modifiers = Modifiers::CTRL;
    }
    f.key(ctrl);
    assert_eq!(f.app.selected.as_ref(), Some(&ids[0]));

    // Share dialog: keys do nothing, Escape closes it.
    f.send(Message::ShareSelected);
    assert!(f.app.share_dialog.is_some());
    f.key(press(char_key("j")));
    f.key(press(Key::Named(Named::Backspace)));
    assert_eq!(f.app.selected.as_ref(), Some(&ids[0]));
    assert_eq!(f.app.emails.len(), 3);
    f.key(press(Key::Named(Named::Escape)));
    assert!(f.app.share_dialog.is_none());

    // Confirm bar.
    f.send(Message::Ask(Confirm::ClearMailbox("Acme".into())));
    f.key(press(Key::Named(Named::Delete)));
    assert_eq!(f.app.emails.len(), 3);
    f.key(press(Key::Named(Named::Escape)));
    assert!(f.app.confirm.is_none());

    // Forward bar.
    f.send(Message::ForwardStart);
    f.key(press(Key::Named(Named::Backspace)));
    f.key(press(char_key("2")));
    assert_eq!(f.app.emails.len(), 3);
    assert_eq!(f.app.tab, Tab::Preview);
    f.send(Message::ForwardCancel);

    // Other pages.
    f.send(Message::Page(Page::Settings));
    f.key(press(Key::Named(Named::Backspace)));
    assert_eq!(f.app.emails.len(), 3);
}

#[test]
fn mail_bursts_reload_once() {
    let mut f = Fixture::new(1);
    let summary = f.app.emails[0].clone();
    f.send(Message::Core(Event::EmailReceived {
        email: summary.clone(),
    }));
    assert!(f.app.reload_pending);
    {
        let _guard = f.runtime.enter();
        f.scratchpad
            .ingest_local(email(7), "Acme", "a@b.c".into(), vec![])
            .unwrap();
    }
    f.send(Message::Core(Event::EmailReceived { email: summary }));
    assert_eq!(f.app.emails.len(), 1, "reloads wait for the flush");
    f.send(Message::Flush);
    assert!(!f.app.reload_pending);
    assert_eq!(f.app.emails.len(), 2);

    // Deleting elsewhere drops the email from the list and the selection.
    let id = f.app.emails[0].id.clone();
    f.send(Message::SelectEmail(id.clone()));
    f.scratchpad
        .delete_emails(std::slice::from_ref(&id))
        .unwrap();
    f.send(Message::Core(Event::EmailsDeleted { ids: vec![id] }));
    assert_eq!(f.app.emails.len(), 1);
    assert!(f.app.selected.is_none() && f.app.detail.is_none());
}

#[test]
fn settings_validation_errors_are_shown() {
    let mut f = Fixture::new(0);
    f.send(Message::Page(Page::Settings));
    f.send(Message::Settings(SettingsMsg::Text(
        Field::SmtpPort,
        "not a port".into(),
    )));
    f.send(Message::SaveSettings);
    let toast = f.app.toast.as_ref().expect("error toast");
    assert!(toast.error && toast.message.contains("SMTP port"));
    assert_eq!(
        f.scratchpad.settings().smtp_port,
        Settings::default().smtp_port
    );

    f.send(Message::Settings(SettingsMsg::Text(
        Field::SmtpPort,
        "2600".into(),
    )));
    f.send(Message::SaveSettings);
    assert!(!f.app.toast.as_ref().unwrap().error);
    assert_eq!(f.scratchpad.settings().smtp_port, 2600);
}

#[test]
fn s3_form_validation_happens_before_saving() {
    let mut f = Fixture::new(0);
    f.send(Message::Page(Page::Cloud));
    f.send(Message::Cloud(CloudMsg::NewConfig));
    f.send(Message::Cloud(CloudMsg::SaveConfig));
    assert!(!f.app.cloud.saving);
    let toast = f.app.toast.as_ref().unwrap();
    assert!(toast.error && toast.message.contains("name"));
    f.send(Message::Cloud(CloudMsg::Form(S3Msg::Text(
        super::forms::S3Field::Name,
        "R2".into(),
    ))));
    f.send(Message::Cloud(CloudMsg::TestForm));
    assert!(f.app.cloud.testing.is_none());
    assert!(f.app.toast.as_ref().unwrap().message.contains("endpoint"));
}

#[test]
fn share_dialog_state() {
    let mut f = Fixture::new(2);
    // Nothing to share.
    f.send(Message::ShareChecked);
    assert!(f.app.share_dialog.is_none());

    let ids = f.ids();
    f.send(Message::ToggleCheck(ids[0].clone(), true));
    f.send(Message::ToggleCheck(ids[1].clone(), true));
    f.send(Message::ShareChecked);
    let dialog = f.app.share_dialog.as_ref().unwrap();
    assert_eq!(dialog.ids, ids, "checked emails in list order");
    assert_eq!(dialog.destination, super::cloud::Destination::Local);

    // A busy dialog can't be closed.
    f.app.share_dialog.as_mut().unwrap().busy = true;
    f.send(Message::Cloud(CloudMsg::Cancel));
    assert!(f.app.share_dialog.is_some());
    f.app.share_dialog.as_mut().unwrap().busy = false;
    f.send(Message::Cloud(CloudMsg::Cancel));
    assert!(f.app.share_dialog.is_none());
}

#[test]
fn forwarding_needs_recipients_and_runs_once() {
    let mut f = Fixture::new(1);
    let id = f.ids()[0].clone();
    f.send(Message::SelectEmail(id));
    f.send(Message::ForwardStart);
    f.send(Message::ForwardInput(" , ; ".into()));
    f.send(Message::ForwardSend);
    assert!(f.app.toast.as_ref().unwrap().error);
    assert!(!f.app.forwarding);
    f.send(Message::ForwardInput("a@example.com".into()));
    f.send(Message::ForwardSend);
    assert!(f.app.forwarding);
    f.send(Message::Forwarded(Err("no relay".into())));
    assert!(!f.app.forwarding && f.app.forward.is_some());
}

#[test]
fn search_and_mailbox_reset_the_list() {
    let mut f = Fixture::new(5);
    f.app.list_scroll = 400.0;
    f.send(Message::Search("Message 3".into()));
    assert_eq!(f.app.emails.len(), 1);
    assert_eq!(f.app.list_scroll, 0.0);
    f.send(Message::Search(String::new()));
    f.send(Message::SelectMailbox(Some("Billing".into())));
    assert_eq!(f.app.emails.len(), 2);
    f.send(Message::CheckAll(true));
    assert_eq!(f.app.checked.len(), 2);
    f.send(Message::SelectMailbox(None));
    assert!(f.app.checked.is_empty());
}

#[test]
fn toasts_expire() {
    let mut f = Fixture::new(0);
    f.send(Message::Copy("x".into()));
    assert!(f.app.toast.is_some());
    f.send(Message::Tick);
    assert!(f.app.toast.is_some());
    f.app.toast.as_mut().unwrap().at -= std::time::Duration::from_secs(4);
    f.send(Message::Tick);
    assert!(f.app.toast.is_none());
}

// ---------------------------------------------------------------- interaction

#[test]
fn click_through_inbox() {
    let mut f = Fixture::new(3);
    f.click("Message 1");
    assert_eq!(
        f.app.detail.as_ref().map(|d| d.summary.subject.as_str()),
        Some("Message 1")
    );
    f.click("Headers");
    assert_eq!(f.app.tab, Tab::Headers);
    f.click("Preview");
    f.click("Mobile");
    assert_eq!(f.app.device, Device::Mobile);
    f.click("Share");
    assert!(f.app.share_dialog.is_some());
    f.click("Cancel");
    assert!(f.app.share_dialog.is_none());
}

#[test]
fn typing_in_search_is_captured() {
    let mut f = Fixture::new(3);
    let messages = {
        let mut ui = simulator(&f.app);
        ui.click(iced_test::selector::id(view::SEARCH)).unwrap();
        // The text field handles the keys, so shortcuts never see them.
        assert_eq!(ui.typewrite("j2"), iced::event::Status::Captured);
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in messages {
        f.send(message);
    }
    assert_eq!(f.app.search, "j2");
    assert!(f.app.selected.is_none());
}

#[test]
fn settings_and_cloud_forms_validate_on_click() {
    let mut f = Fixture::new(0);
    // Tall enough to show the whole settings page without scrolling.
    f.send(Message::Resized(iced::Size::new(1320.0, 2400.0)));
    f.send(Message::Page(Page::Settings));
    f.send(Message::Settings(SettingsMsg::Text(
        Field::WebPort,
        "0".into(),
    )));
    f.click("Save changes");
    assert!(f.app.toast.as_ref().unwrap().message.contains("Web port"));

    f.click("Cloud sharing");
    f.click("Add bucket");
    assert!(f.app.cloud.form.is_some());
    f.click("Save");
    assert!(f.app.toast.as_ref().unwrap().error);
    assert!(f.app.cloud.form.is_some());
}

#[test]
fn smallest_window_renders_every_page() {
    let mut f = Fixture::new(3);
    f.send(Message::Resized(super::MIN_WINDOW));
    let id = f.ids()[0].clone();
    f.send(Message::SelectEmail(id));
    for page in [
        Page::Inbox,
        Page::Setup,
        Page::Shares,
        Page::Cloud,
        Page::Remotes,
        Page::Settings,
    ] {
        f.send(Message::Page(page));
        let mut ui = simulator(&f.app);
        // Snapshots render the whole tree, including layout at this size.
        ui.snapshot(&super::style::light_theme()).unwrap();
    }
}
