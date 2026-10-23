//! Desktop GUI built with iced.
//!
//! The GUI talks to the shared [`Scratchpad`] service directly. iced has no HTML
//! engine, so the Preview tab shows a text rendering in device-sized screens;
//! pixel-perfect HTML previews open in the browser as an exported, self-contained
//! viewer (no web server needed).

mod cloud;
mod forms;
mod icons;
mod preview;
#[cfg(test)]
mod snapshots;
mod style;
#[cfg(test)]
mod tests;
mod view;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Context;
use iced::keyboard::{self, key::Named, Key};
use iced::widget::{operation, scrollable, text_editor};
use iced::{Size, Subscription, Task, Theme};
use scratchpad_core::{
    EmailDetail, EmailSummary, Event, Mailbox, RemoteMailbox, Scratchpad, Settings, Share, Status,
    ThemePreference,
};
use tokio::runtime::Handle;

use cloud::{Cloud, CloudMsg, ShareDialog};
use forms::{RemoteForm, SettingsForm};
use preview::Device;
use scratchpad_web::export::{self, ExportOptions};

static SCRATCHPAD: OnceLock<Arc<Scratchpad>> = OnceLock::new();

/// Initial and minimum window sizes.
const WINDOW: Size = Size::new(1320.0, 840.0);
const MIN_WINDOW: Size = Size::new(960.0, 580.0);
/// The text rendering in the Preview tab is cut off after this many characters;
/// laying out megabytes of text would stall the UI. "Open in browser" shows everything.
const PREVIEW_MAX: usize = 100_000;
/// New-mail notifications are shown at most this often (a test suite can send hundreds).
const NOTIFY_EVERY: Duration = Duration::from_secs(3);
/// Bursts of mail events are folded into one list reload after this delay.
const RELOAD_DELAY: Duration = Duration::from_millis(120);

pub fn run(scratchpad: Arc<Scratchpad>, runtime: Handle) -> anyhow::Result<()> {
    let _ = SCRATCHPAD.set(scratchpad.clone());
    let icon = iced::window::icon::from_file_data(
        include_bytes!("../../../../packaging/icons/scratchpad-256.png"),
        None,
    )
    .ok();
    let result = iced::application(
        move || App::new(scratchpad.clone(), runtime.clone()),
        App::update,
        App::view,
    )
    .title(App::title)
    .subscription(App::subscription)
    .theme(App::theme)
    .settings(view::app_settings())
    .window(iced::window::Settings {
        size: WINDOW,
        min_size: Some(MIN_WINDOW),
        icon,
        // A unified, transparent title bar on macOS; the views reserve `view::TOP_INSET`.
        #[cfg(target_os = "macos")]
        platform_specific: iced::window::settings::PlatformSpecific {
            title_hidden: true,
            titlebar_transparent: true,
            fullsize_content_view: true,
        },
        ..Default::default()
    })
    .run()
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"));
    remove_scratch_dir();
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Inbox,
    Setup,
    Shares,
    Cloud,
    Remotes,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Preview,
    Text,
    Source,
    Raw,
    Headers,
    Attachments,
    Links,
    Spam,
    Inspector,
}

impl Tab {
    pub const ALL: [Tab; 9] = [
        Tab::Preview,
        Tab::Text,
        Tab::Source,
        Tab::Raw,
        Tab::Headers,
        Tab::Attachments,
        Tab::Links,
        Tab::Spam,
        Tab::Inspector,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Preview => "Preview",
            Tab::Text => "Text",
            Tab::Source => "HTML",
            Tab::Raw => "Raw",
            Tab::Headers => "Headers",
            Tab::Attachments => "Attachments",
            Tab::Links => "Links",
            Tab::Spam => "Spam",
            Tab::Inspector => "Inspector",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Confirm {
    ClearMailbox(String),
    DeleteMailbox(String),
    DeleteRemote(String),
    /// Delete the uploaded file and the share.
    DeleteCloudShare(String),
    /// Deleting the file failed (the error): forget the share, keep the file.
    ForgetCloudShare(String, String),
    DeleteS3Config(String),
    ResetVault,
}

#[derive(Debug, Clone)]
pub enum Message {
    Core(Event),
    Resync,
    Tick,
    SystemTheme(iced::theme::Mode),
    ToggleMeta,
    SetTheme(ThemePreference),
    Keyboard(keyboard::Event),
    Resized(Size),
    /// Reload the list after a burst of mail events.
    Flush,

    Page(Page),
    SelectMailbox(Option<String>),
    Search(String),
    SelectEmail(String),
    /// Select the next (`true`) or previous email.
    Navigate(bool),
    ListScrolled(scrollable::Viewport),
    ToggleCheck(String, bool),
    CheckAll(bool),
    MarkCheckedRead,
    DeleteChecked,
    ShareChecked,
    ShareMailbox,
    Ask(Confirm),
    ConfirmYes,
    ConfirmNo,

    Tab(Tab),
    Editor(text_editor::Action),
    Device(Device),
    Rotate,
    OpenPreviewInBrowser,
    ExportSelected,
    ExportChecked,
    /// A background action finished; errors are shown as a toast.
    Done(Result<(), String>),
    OpenUrl(String),
    SaveEml,
    SaveAttachment(usize),
    OpenAttachment(usize),
    FileSaved(Result<Option<String>, String>),
    ShareSelected,
    ForwardStart,
    ForwardInput(String),
    ForwardSend,
    ForwardCancel,
    Forwarded(Result<(), String>),
    Recheck,
    DeleteSelected,
    ToggleRead,

    Copy(String),
    DeleteShare(String),

    Settings(forms::SettingsMsg),
    SaveSettings,

    Remote(forms::RemoteMsg),
    NewRemote,
    EditRemote(String),
    SaveRemote,
    CancelRemote,
    PollRemotes,

    TestMailbox(String),
    SendTestEmail,
    TestEmailSent(Result<(), String>),

    Cloud(CloudMsg),
}

pub struct Toast {
    pub message: String,
    pub error: bool,
    pub at: Instant,
}

pub struct App {
    pub scratchpad: Arc<Scratchpad>,
    runtime: Handle,
    pub status: Status,
    pub settings: Settings,
    pub page: Page,
    pub mailboxes: Vec<Mailbox>,
    pub mailbox: Option<String>,
    pub emails: Vec<EmailSummary>,
    pub search: String,
    pub checked: HashSet<String>,
    pub selected: Option<String>,
    pub detail: Option<EmailDetail>,
    pub preview_text: String,
    pub tab: Tab,
    pub device: Device,
    pub landscape: bool,
    pub editor: text_editor::Content,
    pub forward: Option<String>,
    pub forwarding: bool,
    pub shares: Vec<Share>,
    pub cloud: Cloud,
    pub share_dialog: Option<ShareDialog>,
    pub remotes: Vec<RemoteMailbox>,
    pub remote_form: Option<RemoteForm>,
    pub settings_form: SettingsForm,
    pub test_mailbox: String,
    pub confirm: Option<Confirm>,
    pub toast: Option<Toast>,
    pub system_dark: bool,
    pub show_meta: bool,
    /// Current window size, for layouts that adapt to narrow windows.
    pub window: Size,
    /// Scroll offset and visible height of the email list (it only builds visible rows).
    pub list_scroll: f32,
    pub list_height: f32,
    pub sending_test: bool,
    reload_pending: bool,
    last_notification: Option<Instant>,
}

impl App {
    fn new(scratchpad: Arc<Scratchpad>, runtime: Handle) -> (Self, Task<Message>) {
        let settings = scratchpad.settings();
        let cloud = Cloud::load(&scratchpad);
        let mut app = Self {
            status: scratchpad.status(),
            settings_form: SettingsForm::new(&settings),
            settings,
            scratchpad,
            runtime,
            page: Page::Inbox,
            mailboxes: vec![],
            mailbox: None,
            emails: vec![],
            search: String::new(),
            checked: HashSet::new(),
            selected: None,
            detail: None,
            preview_text: String::new(),
            tab: Tab::Preview,
            device: Device::Fit,
            landscape: false,
            editor: text_editor::Content::new(),
            forward: None,
            forwarding: false,
            shares: vec![],
            cloud,
            share_dialog: None,
            remotes: vec![],
            remote_form: None,
            test_mailbox: "Test".into(),
            confirm: None,
            toast: None,
            system_dark: true,
            show_meta: false,
            window: WINDOW,
            list_scroll: 0.0,
            list_height: 0.0,
            sending_test: false,
            reload_pending: false,
            last_notification: None,
        };
        app.reload_all();
        (app, iced::system::theme().map(Message::SystemTheme))
    }

    fn title(&self) -> String {
        let unread: u64 = self.mailboxes.iter().map(|m| m.unread).sum();
        if unread > 0 {
            format!("Scratchpad ({unread})")
        } else {
            "Scratchpad".into()
        }
    }

    fn theme(&self) -> Option<Theme> {
        let dark = match self.settings.theme {
            ThemePreference::System => self.system_dark,
            ThemePreference::Light => false,
            ThemePreference::Dark => true,
        };
        Some(if dark {
            style::dark_theme()
        } else {
            style::light_theme()
        })
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            Subscription::run(core_events),
            keyboard::listen().map(Message::Keyboard),
            iced::system::theme_changes().map(Message::SystemTheme),
            iced::window::resize_events().map(|(_, size)| Message::Resized(size)),
        ];
        if self.toast.is_some() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(500)).map(|_| Message::Tick));
        }
        Subscription::batch(subscriptions)
    }

    // ---- helpers -------------------------------------------------------------

    fn toast(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast {
            message: message.into(),
            error: false,
            at: Instant::now(),
        });
    }

    fn error(&mut self, message: impl std::fmt::Display) {
        self.toast = Some(Toast {
            message: message.to_string(),
            error: true,
            at: Instant::now(),
        });
    }

    fn reload_all(&mut self) {
        self.status = self.scratchpad.status();
        self.reload_mailboxes();
        self.reload_emails();
        self.shares = self.scratchpad.shares().unwrap_or_default();
        self.remotes = self.scratchpad.remotes().unwrap_or_default();
        self.reload_cloud();
    }

    fn reload_mailboxes(&mut self) {
        match self.scratchpad.mailboxes() {
            Ok(mailboxes) => self.mailboxes = mailboxes,
            Err(e) => self.error(e),
        }
        if let Some(name) = &self.mailbox {
            if !self.mailboxes.iter().any(|m| &m.name == name) {
                self.mailbox = None;
            }
        }
    }

    fn reload_emails(&mut self) {
        match self
            .scratchpad
            .emails(self.mailbox.as_deref(), Some(&self.search))
        {
            Ok(emails) => self.emails = emails,
            Err(e) => self.error(e),
        }
        let ids: HashSet<&String> = self.emails.iter().map(|e| &e.id).collect();
        self.checked.retain(|id| ids.contains(id));
        if let Some(id) = &self.selected {
            if !ids.contains(id) {
                self.selected = None;
                self.detail = None;
            }
        }
    }

    fn reload_detail(&mut self) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        match self.scratchpad.email(&id) {
            Ok(Some(detail)) => {
                let changed_email = self.detail.as_ref().map(|d| &d.summary.id) != Some(&id);
                let text = match &detail.html {
                    Some(html) => scratchpad_core::parse::html_to_text(html, 100),
                    None => detail.text.clone().unwrap_or_default(),
                };
                self.preview_text = truncate_preview(text);
                self.detail = Some(detail);
                if changed_email {
                    if self.tab == Tab::Inspector
                        && self
                            .detail
                            .as_ref()
                            .is_some_and(|d| d.view_inspector.is_none())
                    {
                        self.tab = Tab::Preview;
                    }
                    self.fill_editor();
                }
            }
            Ok(None) => {
                self.selected = None;
                self.detail = None;
            }
            Err(e) => self.error(e),
        }
    }

    /// Loads the text for the current tab into the (read-only, selectable) editor.
    fn fill_editor(&mut self) {
        let Some(detail) = &self.detail else { return };
        let text = match self.tab {
            Tab::Text => detail
                .text
                .clone()
                .unwrap_or_else(|| "This email has no plain-text part.".into()),
            Tab::Source => detail
                .html
                .clone()
                .unwrap_or_else(|| "This email has no HTML part.".into()),
            Tab::Raw => self
                .scratchpad
                .raw(&detail.summary.id)
                .ok()
                .flatten()
                .map(|raw| String::from_utf8_lossy(&raw).replace("\r\n", "\n"))
                .unwrap_or_default(),
            Tab::Inspector => detail
                .view_inspector
                .as_ref()
                .and_then(|v| v.view_source.clone())
                .unwrap_or_default(),
            _ => return,
        };
        self.editor = text_editor::Content::with_text(&text);
    }

    fn select(&mut self, id: String) {
        self.page = Page::Inbox;
        self.selected = Some(id.clone());
        self.forward = None;
        self.reload_detail();
        if self.emails.iter().any(|e| e.id == id && !e.read) {
            self.set_read(std::slice::from_ref(&id), true);
        }
    }

    /// Marks emails (un)read and updates the list and counts right away.
    fn set_read(&mut self, ids: &[String], read: bool) {
        if let Err(e) = self.scratchpad.mark_read(ids, read) {
            self.error(e);
            return;
        }
        self.reload_mailboxes();
        let ids: HashSet<&String> = ids.iter().collect();
        for email in self.emails.iter_mut().filter(|e| ids.contains(&e.id)) {
            email.read = read;
        }
        if let Some(detail) = self.detail.as_mut() {
            if ids.contains(&detail.summary.id) {
                detail.summary.read = read;
            }
        }
    }

    /// Moves the selection one email down or up and scrolls it into view.
    fn navigate(&mut self, down: bool) -> Task<Message> {
        let index = self
            .selected
            .as_ref()
            .and_then(|id| self.emails.iter().position(|e| &e.id == id));
        let next = match (index, down) {
            (Some(i), true) => i + 1,
            (Some(i), false) => match i.checked_sub(1) {
                Some(i) => i,
                None => return Task::none(),
            },
            (None, _) => 0,
        };
        let Some(id) = self.emails.get(next).map(|e| e.id.clone()) else {
            return Task::none();
        };
        self.select(id);
        self.reveal(next)
    }

    /// Scrolls the email list so that row `index` is visible.
    fn reveal(&mut self, index: usize) -> Task<Message> {
        let height = if self.list_height > 0.0 {
            self.list_height
        } else {
            // Not scrolled yet: estimate from the window.
            (self.window.height - view::LIST_CHROME).max(view::ROW_STRIDE)
        };
        match view::reveal_offset(index, self.list_scroll, height) {
            Some(y) => {
                self.list_scroll = y;
                operation::scroll_to(view::LIST, scrollable::AbsoluteOffset { x: 0.0, y })
            }
            None => Task::none(),
        }
    }

    /// Scrolls the list back to the top (new mailbox or search).
    fn list_to_top(&mut self) -> Task<Message> {
        self.list_scroll = 0.0;
        operation::snap_to(view::LIST, scrollable::RelativeOffset::START)
    }

    /// Reloads the list shortly, folding bursts of events into one reload.
    fn schedule_reload(&mut self) -> Task<Message> {
        if self.reload_pending {
            return Task::none();
        }
        self.reload_pending = true;
        let delay = self.runtime.spawn(tokio::time::sleep(RELOAD_DELAY));
        Task::perform(
            async move {
                let _ = delay.await;
            },
            |_| Message::Flush,
        )
    }

    /// Runs blocking work (disk, parsing) off the UI thread; errors become a toast.
    fn blocking(
        &self,
        context: &'static str,
        work: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
    ) -> Task<Message> {
        let task = self.runtime.spawn_blocking(work);
        Task::perform(
            async move {
                task.await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| format!("{context}: {e:#}"))
            },
            Message::Done,
        )
    }

    fn delete(&mut self, ids: Vec<String>) {
        let index = self
            .selected
            .as_ref()
            .and_then(|id| self.emails.iter().position(|e| &e.id == id));
        if let Err(e) = self.scratchpad.delete_emails(&ids) {
            self.error(e);
            return;
        }
        let removed_selected = self.selected.as_ref().is_some_and(|id| ids.contains(id));
        self.reload_emails();
        self.reload_mailboxes();
        if removed_selected {
            let next = index
                .and_then(|i| self.emails.get(i.min(self.emails.len().saturating_sub(1))))
                .map(|e| e.id.clone());
            match next {
                Some(id) => self.select(id),
                None => {
                    self.selected = None;
                    self.detail = None;
                }
            }
        }
    }

    fn shared(&mut self, result: anyhow::Result<Share>) -> Task<Message> {
        match result {
            Ok(share) => {
                self.shares = self.scratchpad.shares().unwrap_or_default();
                match self.scratchpad.share_url(&share.token) {
                    Some(url) => {
                        self.toast("Share link copied to clipboard");
                        iced::clipboard::write(url)
                    }
                    None => {
                        self.toast("Share created. Enable the web interface to get a link.");
                        Task::none()
                    }
                }
            }
            Err(e) => {
                self.error(e);
                Task::none()
            }
        }
    }

    /// Exports `ids` as one .html file, asking where to save it first.
    fn export(&self, title: String, ids: Vec<String>) -> Task<Message> {
        if ids.is_empty() {
            return Task::none();
        }
        let scratchpad = self.scratchpad.clone();
        let runtime = self.runtime.clone();
        let name = export::file_name(&title);
        Task::perform(
            async move {
                let Some(handle) = rfd::AsyncFileDialog::new()
                    .set_file_name(&name)
                    .add_filter("HTML", &["html"])
                    .save_file()
                    .await
                else {
                    return Ok(None);
                };
                let path = handle.path().to_path_buf();
                let html = runtime
                    .spawn_blocking(move || {
                        export::render(&scratchpad, &ids, Some(&title), &ExportOptions::default())
                    })
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| format!("Export failed: {e:#}"))?;
                std::fs::write(&path, html).map_err(|e| e.to_string())?;
                Ok(Some(path.display().to_string()))
            },
            Message::FileSaved,
        )
    }

    /// Ids of the checked emails, in list order.
    fn checked_ids(&self) -> Vec<String> {
        self.emails
            .iter()
            .filter(|e| self.checked.contains(&e.id))
            .map(|e| e.id.clone())
            .collect()
    }

    fn email_url(&self, path: &str) -> Option<String> {
        self.scratchpad
            .web_url()
            .map(|base| format!("{base}{path}"))
    }

    fn notify(&mut self, email: &EmailSummary) {
        if !self.settings.notifications
            || self
                .last_notification
                .is_some_and(|at| at.elapsed() < NOTIFY_EVERY)
        {
            return;
        }
        self.last_notification = Some(Instant::now());
        let summary = email.subject.clone();
        let body = format!("{}\n{}", email.from, email.mailbox);
        std::thread::spawn(move || {
            init_notifications();
            let _ = notify_rust::Notification::new()
                .appname("Scratchpad")
                .summary(&summary)
                .body(&body)
                .show();
        });
    }

    // ---- update ----------------------------------------------------------------

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Core(event) => return self.on_event(event),
            Message::Resync => self.reload_all(),
            Message::Flush => {
                self.reload_pending = false;
                self.reload_mailboxes();
                self.reload_emails();
            }
            Message::Resized(size) => self.window = size,
            Message::SystemTheme(mode) => self.system_dark = mode != iced::theme::Mode::Light,
            Message::ToggleMeta => self.show_meta = !self.show_meta,
            Message::SetTheme(theme) => {
                let mut settings = self.scratchpad.settings();
                settings.theme = theme;
                match self.scratchpad.update_settings(settings) {
                    Ok(()) => {
                        self.settings = self.scratchpad.settings();
                        self.settings_form.draft.theme = theme;
                    }
                    Err(e) => self.error(e),
                }
            }
            Message::Tick => {
                if self.toast.as_ref().is_some_and(|t| {
                    t.at.elapsed() > Duration::from_secs(if t.error { 6 } else { 3 })
                }) {
                    self.toast = None;
                }
            }
            Message::Keyboard(event) => return self.on_key(event),

            Message::Page(page) => {
                self.page = page;
                match page {
                    Page::Settings => {
                        self.settings_form = SettingsForm::new(&self.scratchpad.settings())
                    }
                    Page::Shares => self.shares = self.scratchpad.shares().unwrap_or_default(),
                    Page::Cloud => self.reload_cloud(),
                    Page::Remotes => self.remotes = self.scratchpad.remotes().unwrap_or_default(),
                    Page::Setup | Page::Inbox => self.status = self.scratchpad.status(),
                }
            }
            Message::SelectMailbox(mailbox) => {
                self.page = Page::Inbox;
                self.mailbox = mailbox;
                self.checked.clear();
                self.reload_emails();
                return self.list_to_top();
            }
            Message::Search(search) => {
                self.search = search;
                self.reload_emails();
                return self.list_to_top();
            }
            Message::SelectEmail(id) => self.select(id),
            Message::Navigate(down) => return self.navigate(down),
            Message::ListScrolled(viewport) => {
                self.list_scroll = viewport.absolute_offset().y;
                self.list_height = viewport.bounds().height;
            }
            Message::ToggleCheck(id, checked) => {
                if checked {
                    self.checked.insert(id);
                } else {
                    self.checked.remove(&id);
                }
            }
            Message::CheckAll(all) => {
                self.checked = if all {
                    self.emails.iter().map(|e| e.id.clone()).collect()
                } else {
                    HashSet::new()
                };
            }
            Message::MarkCheckedRead => {
                let ids: Vec<String> = self.checked.iter().cloned().collect();
                self.set_read(&ids, true);
            }
            Message::DeleteChecked => {
                let ids: Vec<String> = self.checked.drain().collect();
                self.delete(ids);
            }
            Message::ShareChecked => {
                let ids = self.checked_ids();
                let title = self
                    .mailbox
                    .clone()
                    .unwrap_or_else(|| "Shared emails".into());
                return self.open_share_dialog(title, ids, None);
            }
            Message::ShareMailbox => {
                let ids: Vec<String> = self.emails.iter().map(|e| e.id.clone()).collect();
                let title = self.mailbox.clone().unwrap_or_else(|| "All emails".into());
                return self.open_share_dialog(title, ids, self.mailbox.clone());
            }
            Message::ExportChecked => {
                let ids = self.checked_ids();
                let title = self
                    .mailbox
                    .clone()
                    .unwrap_or_else(|| format!("{} emails", ids.len()));
                return self.export(title, ids);
            }
            Message::ExportSelected => {
                if let Some(detail) = &self.detail {
                    let (title, id) = (detail.summary.subject.clone(), detail.summary.id.clone());
                    return self.export(title, vec![id]);
                }
            }
            Message::Ask(confirm) => self.confirm = Some(confirm),
            Message::ConfirmNo => self.confirm = None,
            Message::ConfirmYes => {
                let result = match self.confirm.take() {
                    Some(Confirm::ClearMailbox(name)) => self.scratchpad.clear_mailbox(&name),
                    Some(Confirm::DeleteMailbox(name)) => {
                        self.mailbox = None;
                        self.scratchpad.delete_mailbox(&name)
                    }
                    Some(Confirm::DeleteRemote(id)) => self.scratchpad.delete_remote(&id, true),
                    Some(confirm) => return self.cloud_confirmed(confirm),
                    None => Ok(()),
                };
                if let Err(e) = result {
                    self.error(e);
                }
                self.reload_all();
            }

            Message::Tab(tab) => {
                self.tab = tab;
                self.fill_editor();
            }
            Message::Editor(action) => {
                // Read-only: allow selection and navigation, ignore edits.
                if !action.is_edit() {
                    self.editor.perform(action);
                }
            }
            Message::Device(device) => self.device = device,
            Message::Rotate => self.landscape = !self.landscape,
            Message::OpenPreviewInBrowser => {
                // Export the real viewer to a temp file, opened on the current device.
                let Some(id) = self.selected.clone() else {
                    return Task::none();
                };
                let scratchpad = self.scratchpad.clone();
                let options = ExportOptions {
                    device: Some(self.device.id().to_string()),
                    ..ExportOptions::default()
                };
                return self.blocking("Could not open the preview", move || {
                    let html =
                        export::render(&scratchpad, std::slice::from_ref(&id), None, &options)?;
                    let path = scratch_file("preview.html")?;
                    std::fs::write(&path, html)?;
                    open::that_detached(&path)?;
                    Ok(())
                });
            }
            Message::Done(result) => {
                if let Err(e) = result {
                    self.error(e);
                }
            }
            Message::OpenUrl(url) => {
                if let Err(e) = open_url(&url) {
                    self.error(e);
                }
            }
            Message::SaveEml => {
                let Some(detail) = &self.detail else {
                    return Task::none();
                };
                let id = detail.summary.id.clone();
                let name = format!("{}.eml", sanitize(&detail.summary.subject));
                let scratchpad = self.scratchpad.clone();
                return self.save_in_background(move || {
                    let raw = scratchpad.raw(&id)?.context("The email no longer exists")?;
                    Ok((name, raw))
                });
            }
            Message::SaveAttachment(index) => {
                let Some(id) = self.selected.clone() else {
                    return Task::none();
                };
                let scratchpad = self.scratchpad.clone();
                return self.save_in_background(move || {
                    let (meta, bytes) = scratchpad
                        .attachment(&id, index)?
                        .context("Attachment not found")?;
                    Ok((sanitize(&meta.filename), bytes))
                });
            }
            Message::OpenAttachment(index) => {
                let Some(id) = self.selected.clone() else {
                    return Task::none();
                };
                let scratchpad = self.scratchpad.clone();
                return self.blocking("Could not open the attachment", move || {
                    let (meta, bytes) = scratchpad
                        .attachment(&id, index)?
                        .context("Attachment not found")?;
                    if is_executable(&sanitize(&meta.filename)) {
                        anyhow::bail!(
                            "“{}” could run code on this computer. Save it and open it yourself if you trust it.",
                            meta.filename
                        );
                    }
                    let path = scratch_file(&meta.filename)?;
                    std::fs::write(&path, bytes)?;
                    open::that_detached(&path)?;
                    Ok(())
                });
            }
            Message::FileSaved(result) => match result {
                Ok(Some(path)) => self.toast(format!("Saved to {path}")),
                Ok(None) => {}
                Err(e) => self.error(e),
            },
            Message::ShareSelected => {
                let Some(detail) = &self.detail else {
                    return Task::none();
                };
                let (title, id) = (detail.summary.subject.clone(), detail.summary.id.clone());
                return self.open_share_dialog(title, vec![id], None);
            }
            Message::ForwardStart => self.forward = Some(String::new()),
            Message::ForwardInput(value) => self.forward = Some(value),
            Message::ForwardCancel => self.forward = None,
            Message::ForwardSend => {
                let (Some(id), Some(input)) = (self.selected.clone(), self.forward.clone()) else {
                    return Task::none();
                };
                if self.forwarding {
                    return Task::none();
                }
                let recipients: Vec<String> = input
                    .split([',', ';', ' '])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                if recipients.is_empty() {
                    self.error("Enter at least one recipient");
                    return Task::none();
                }
                self.forwarding = true;
                let scratchpad = self.scratchpad.clone();
                let task = self.runtime.spawn(async move {
                    scratchpad
                        .forward(&id, &recipients)
                        .await
                        .map_err(|e| format!("{e:#}"))
                });
                return Task::perform(
                    async move { task.await.unwrap_or_else(|e| Err(e.to_string())) },
                    Message::Forwarded,
                );
            }
            Message::Forwarded(result) => {
                self.forwarding = false;
                match result {
                    Ok(()) => {
                        self.forward = None;
                        self.toast("Email forwarded");
                    }
                    Err(e) => self.error(format!("Forwarding failed: {e}")),
                }
            }
            Message::Recheck => {
                if let Some(id) = &self.selected {
                    let _guard = self.runtime.enter();
                    self.scratchpad.recheck(id);
                    self.toast("Running link checker and spam report…");
                }
            }
            Message::DeleteSelected => {
                if let Some(id) = self.selected.clone() {
                    self.delete(vec![id]);
                }
            }
            Message::ToggleRead => {
                if let Some(detail) = &self.detail {
                    let (id, read) = (detail.summary.id.clone(), !detail.summary.read);
                    self.set_read(&[id], read);
                }
            }

            Message::Copy(text) => {
                self.toast("Copied to clipboard");
                return iced::clipboard::write(text);
            }
            Message::DeleteShare(token) => {
                if let Err(e) = self.scratchpad.delete_share(&token) {
                    self.error(e);
                }
                self.shares = self.scratchpad.shares().unwrap_or_default();
            }

            Message::Settings(msg) => self.settings_form.update(msg),
            Message::SaveSettings => match self.settings_form.build() {
                Ok(settings) => match self.scratchpad.update_settings(settings) {
                    Ok(()) => {
                        self.settings = self.scratchpad.settings();
                        self.settings_form = SettingsForm::new(&self.settings);
                        self.toast("Settings saved");
                    }
                    Err(e) => self.error(e),
                },
                Err(e) => self.error(e),
            },

            Message::Remote(msg) => {
                if let Some(form) = &mut self.remote_form {
                    form.update(msg);
                }
            }
            Message::NewRemote => self.remote_form = Some(RemoteForm::default()),
            Message::EditRemote(id) => {
                self.remote_form = self
                    .remotes
                    .iter()
                    .find(|r| r.id == id)
                    .map(RemoteForm::from_remote);
            }
            Message::CancelRemote => self.remote_form = None,
            Message::SaveRemote => {
                if let Some(form) = &self.remote_form {
                    match form
                        .build()
                        .and_then(|remote| self.scratchpad.save_remote(remote))
                    {
                        Ok(_) => {
                            self.remote_form = None;
                            self.remotes = self.scratchpad.remotes().unwrap_or_default();
                            self.reload_mailboxes();
                            self.toast("Remote mailbox saved");
                        }
                        Err(e) => self.error(e),
                    }
                }
            }
            Message::PollRemotes => {
                self.scratchpad.poll_remotes_now();
                self.toast("Checking remote mailboxes…");
            }

            Message::TestMailbox(name) => self.test_mailbox = name,
            Message::SendTestEmail => {
                if self.sending_test {
                    return Task::none();
                }
                self.sending_test = true;
                let scratchpad = self.scratchpad.clone();
                let mailbox = if self.test_mailbox.trim().is_empty() {
                    "Test".to_string()
                } else {
                    self.test_mailbox.clone()
                };
                let task = self.runtime.spawn(async move {
                    scratchpad
                        .send_test_email(&mailbox)
                        .await
                        .map_err(|e| format!("{e:#}"))
                });
                return Task::perform(
                    async move { task.await.unwrap_or_else(|e| Err(e.to_string())) },
                    Message::TestEmailSent,
                );
            }
            Message::TestEmailSent(result) => {
                self.sending_test = false;
                match result {
                    Ok(()) => self.toast("Test email sent"),
                    Err(e) => self.error(format!("Could not send test email: {e}")),
                }
            }

            Message::Cloud(msg) => return self.update_cloud(msg),
        }
        Task::none()
    }

    fn on_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::EmailReceived { email } => {
                self.notify(&email);
                return self.schedule_reload();
            }
            Event::EmailUpdated { id } => {
                if self.selected.as_deref() == Some(&id) {
                    self.reload_detail();
                }
                if self.emails.iter().any(|e| e.id == id) {
                    return self.schedule_reload();
                }
            }
            Event::EmailsDeleted { ids } => {
                let ids: HashSet<&String> = ids.iter().collect();
                if self.emails.iter().any(|e| ids.contains(&e.id)) {
                    self.reload_emails();
                }
            }
            Event::MailboxesChanged => self.reload_mailboxes(),
            Event::SharesChanged => self.shares = self.scratchpad.shares().unwrap_or_default(),
            Event::RemotesChanged => self.remotes = self.scratchpad.remotes().unwrap_or_default(),
            Event::SettingsChanged => self.settings = self.scratchpad.settings(),
            Event::CloudChanged => self.reload_cloud(),
            Event::SmtpStatus { .. } => self.status = self.scratchpad.status(),
        }
        Task::none()
    }

    /// Keyboard shortcuts for the inbox: arrows or j/k move through the list,
    /// Backspace/Delete removes the selected email and 1-9 switch tabs.
    ///
    /// Only keys no widget handled arrive here, so typing in a text field never
    /// triggers them. Shortcuts are off while a dialog, the confirm bar or the
    /// forward bar is open.
    fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        let keyboard::Event::KeyPressed {
            key,
            modifiers,
            physical_key,
            repeat,
            ..
        } = event
        else {
            return Task::none();
        };
        if key == Key::Named(Named::Escape) {
            if self.share_dialog.is_some() {
                self.close_share_dialog();
            } else if self.confirm.is_some() {
                self.confirm = None;
            }
            return Task::none();
        }
        if self.share_dialog.is_some()
            || self.confirm.is_some()
            || self.page != Page::Inbox
            || self.forward.is_some()
        {
            return Task::none();
        }
        if modifiers.command() || modifiers.control() || modifiers.alt() || modifiers.logo() {
            return Task::none();
        }
        match key.as_ref() {
            // Text fields don't use up/down, so make sure the search field isn't focused.
            Key::Named(Named::ArrowDown | Named::ArrowUp) => {
                let down = key == Key::Named(Named::ArrowDown);
                return operation::is_focused(view::SEARCH).then(move |focused| {
                    if focused {
                        Task::none()
                    } else {
                        Task::done(Message::Navigate(down))
                    }
                });
            }
            // Holding the key down must not delete one email after another.
            Key::Named(Named::Backspace | Named::Delete) if !repeat => {
                if let Some(id) = self.selected.clone() {
                    self.delete(vec![id]);
                    let index = self
                        .selected
                        .as_ref()
                        .and_then(|id| self.emails.iter().position(|e| &e.id == id));
                    if let Some(index) = index {
                        return self.reveal(index);
                    }
                }
            }
            Key::Character(_) => match shortcut_char(&key, physical_key) {
                Some('j') => return self.navigate(true),
                Some('k') => return self.navigate(false),
                Some(digit @ '1'..='9') => {
                    // 1-9 switch between the visible detail tabs.
                    let has_inspector = self
                        .detail
                        .as_ref()
                        .is_some_and(|d| d.view_inspector.is_some());
                    let index = usize::from(digit as u8 - b'1');
                    if let Some(tab) = Tab::ALL
                        .into_iter()
                        .filter(|t| *t != Tab::Inspector || has_inspector)
                        .nth(index)
                    {
                        self.tab = tab;
                        self.fill_editor();
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Task::none()
    }

    /// Reads the file to save on a background thread, then asks where to save it.
    fn save_in_background(
        &self,
        read: impl FnOnce() -> anyhow::Result<(String, Vec<u8>)> + Send + 'static,
    ) -> Task<Message> {
        let task = self.runtime.spawn_blocking(read);
        Task::perform(
            async move {
                let (name, bytes) = task
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| format!("{e:#}"))?;
                save_file(name, bytes).await
            },
            Message::FileSaved,
        )
    }

    fn view(&self) -> iced::Element<'_, Message> {
        view::view(self)
    }
}

fn core_events() -> impl iced::futures::Stream<Item = Message> {
    let receiver = SCRATCHPAD
        .get()
        .expect("Scratchpad initialised")
        .subscribe();
    iced::futures::stream::unfold(receiver, |mut receiver| async move {
        use tokio::sync::broadcast::error::RecvError;
        match receiver.recv().await {
            Ok(event) => Some((Message::Core(event), receiver)),
            Err(RecvError::Lagged(_)) => Some((Message::Resync, receiver)),
            Err(RecvError::Closed) => None,
        }
    })
}

/// macOS notifications need a bundle identifier. Without one, notify-rust asks the
/// user to pick an application, so fall back to Terminal when running unbundled.
fn init_notifications() {
    #[cfg(target_os = "macos")]
    {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let bundled = std::env::current_exe()
                .map(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS"))
                .unwrap_or(false);
            let id = if bundled {
                scratchpad_core::BUNDLE_ID
            } else {
                "com.apple.Terminal"
            };
            let _ = notify_rust::set_application(id);
        });
    }
}

/// Opens web links in the browser. Links come from untrusted emails, so other schemes
/// (`file:`, custom app handlers, …) are refused.
fn open_url(url: &str) -> Result<(), String> {
    let scheme = url
        .split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    if !matches!(scheme.as_deref(), Some("http" | "https" | "mailto")) {
        return Err(format!("Only web links can be opened: {url}"));
    }
    open::that_detached(url).map_err(|e| format!("Could not open {url}: {e}"))
}

/// The key as a latin character (`j`, `k`, `1`…), independent of the keyboard layout.
fn shortcut_char(key: &Key, physical_key: keyboard::key::Physical) -> Option<char> {
    use keyboard::key::{Code, Physical};
    let digit = match physical_key {
        Physical::Code(Code::Digit1 | Code::Numpad1) => Some('1'),
        Physical::Code(Code::Digit2 | Code::Numpad2) => Some('2'),
        Physical::Code(Code::Digit3 | Code::Numpad3) => Some('3'),
        Physical::Code(Code::Digit4 | Code::Numpad4) => Some('4'),
        Physical::Code(Code::Digit5 | Code::Numpad5) => Some('5'),
        Physical::Code(Code::Digit6 | Code::Numpad6) => Some('6'),
        Physical::Code(Code::Digit7 | Code::Numpad7) => Some('7'),
        Physical::Code(Code::Digit8 | Code::Numpad8) => Some('8'),
        Physical::Code(Code::Digit9 | Code::Numpad9) => Some('9'),
        _ => None,
    };
    // AZERTY layouts need Shift for digits; the physical key still says which one.
    digit.or_else(|| key.to_latin(physical_key).map(|c| c.to_ascii_lowercase()))
}

/// Cuts very long previews (see [`PREVIEW_MAX`]).
fn truncate_preview(text: String) -> String {
    match text.char_indices().nth(PREVIEW_MAX) {
        Some((end, _)) => format!(
            "{}\n\n… The preview is cut off here. Use Open in browser to see the whole email.",
            &text[..end]
        ),
        None => text,
    }
}

/// File types that run code when opened.
fn is_executable(name: &str) -> bool {
    const RISKY: &[&str] = &[
        "app",
        "applescript",
        "bat",
        "cmd",
        "com",
        "command",
        "cpl",
        "desktop",
        "exe",
        "hta",
        "jar",
        "js",
        "jse",
        "lnk",
        "msc",
        "msi",
        "msp",
        "pif",
        "pkg",
        "ps1",
        "reg",
        "scf",
        "scpt",
        "scr",
        "sh",
        "terminal",
        "url",
        "vb",
        "vbe",
        "vbs",
        "workflow",
        "ws",
        "wsf",
        "wsh",
    ];
    let name = name.trim_end_matches(['.', ' ']);
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| RISKY.contains(&ext.to_ascii_lowercase().as_str()))
}

/// A private directory for previews and opened attachments, created on first use
/// (owner-only, with a fresh name so nobody else can plant files in it) and removed
/// when the app quits.
static SCRATCH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn scratch_dir() -> std::io::Result<PathBuf> {
    let mut scratch = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dir) = scratch.as_ref() {
        return Ok(dir.clone());
    }
    let base = std::env::temp_dir();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let mut attempt = 0u32;
    let dir = loop {
        let dir = base.join(format!(
            "scratchpad-{}-{:x}",
            std::process::id(),
            seed.wrapping_add(u128::from(attempt) * 7919)
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&dir) {
            Ok(()) => break dir,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < 16 => attempt += 1,
            Err(e) => return Err(e),
        }
    };
    remove_stale_scratch_dirs(&base, &dir);
    *scratch = Some(dir.clone());
    Ok(dir)
}

/// A new file named `name` in its own folder in the scratch directory, so opening the
/// same attachment twice doesn't overwrite a file another app still has open.
fn scratch_file(name: &str) -> std::io::Result<PathBuf> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = scratch_dir()?.join(COUNTER.fetch_add(1, Ordering::Relaxed).to_string());
    std::fs::create_dir(&dir)?;
    Ok(dir.join(sanitize(name)))
}

fn remove_scratch_dir() {
    let dir = SCRATCH.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(dir) = dir {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Removes scratch directories older than a day that a crashed run left behind.
fn remove_stale_scratch_dirs(base: &Path, current: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    #[cfg(unix)]
    let owner = std::fs::metadata(current).map(|m| std::os::unix::fs::MetadataExt::uid(&m));
    for entry in entries.flatten() {
        let path = entry.path();
        if path == current
            || !entry
                .file_name()
                .to_string_lossy()
                .starts_with("scratchpad-")
        {
            continue;
        }
        // `symlink_metadata`: never follow a link out of the temp directory.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        #[cfg(unix)]
        if owner.as_ref().ok() != Some(&std::os::unix::fs::MetadataExt::uid(&meta)) {
            continue;
        }
        let old = meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(24 * 3600));
        if meta.is_dir() && old {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// A safe file name: no path separators or leading dots, at most 80 characters
/// (keeping the extension), and not a reserved Windows device name.
fn sanitize(name: &str) -> String {
    const MAX: usize = 80;
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Windows drops trailing dots and spaces; leading dots hide files on Unix.
    let cleaned = cleaned.trim_matches([' ', '.']);
    let cleaned = if cleaned.chars().count() > MAX {
        match cleaned.rsplit_once('.') {
            Some((stem, ext)) if !ext.is_empty() && ext.chars().count() <= 10 => {
                let stem: String = stem.chars().take(MAX - ext.chars().count() - 1).collect();
                format!("{}.{ext}", stem.trim_end())
            }
            _ => cleaned
                .chars()
                .take(MAX)
                .collect::<String>()
                .trim()
                .to_string(),
        }
    } else {
        cleaned.to_string()
    };
    if cleaned.is_empty() {
        return "email".into();
    }
    let stem = cleaned.split('.').next().unwrap_or_default();
    const RESERVED: &[&str] = &["con", "prn", "aux", "nul"];
    let lower = stem.to_ascii_lowercase();
    let reserved = RESERVED.contains(&lower.as_str())
        || ((lower.starts_with("com") || lower.starts_with("lpt"))
            && lower.len() == 4
            && lower.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

async fn save_file(name: String, bytes: Vec<u8>) -> Result<Option<String>, String> {
    let Some(handle) = rfd::AsyncFileDialog::new()
        .set_file_name(&name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = handle.path().to_path_buf();
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}
