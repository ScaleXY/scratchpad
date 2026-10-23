//! Cloud sharing: the share dialog, cloud shares, S3 configs and the credential vault.

use iced::widget::{
    button, column, container, opaque, row, scrollable, space, text, text_input, toggler, Column,
    Row,
};
use iced::{Alignment, Element, Fill, Length, Task, Theme};
use scratchpad_core::cloud::DEFAULT_EXPIRY_SECS;
use scratchpad_core::s3::MAX_PRESIGN_SECS;
use scratchpad_core::vault::VaultStatus;
use scratchpad_core::{CloudShare, CredentialStorage, S3Check, S3Config, S3Mode, Visibility};
use scratchpad_web::export::{CloudShareRequest, ExportOptions};

use super::forms::{
    S3Field, S3Form, S3Msg, VaultAction, VaultField, VaultForm, MIN_MASTER_PASSWORD,
};
use super::icons::Icon;
use super::style::{self, Kind, Tone};
use super::view::{
    btn, chip, chip_icon, danger_c, empty, faint, fg, format_full, format_size, format_time,
    hairline, icon, icon_btn, muted, muted_c, page_head, section, setting, tile_icon, tx, MEDIUM,
    MONO, SEMIBOLD,
};
use super::{App, Confirm, Message, Page};

/// Focused when the share dialog opens.
const SHARE_TITLE: &str = "share-title";
/// `Cloud::checks` key for results of the form being edited.
const FORM: &str = "";
const HOUR: u64 = 3600;
const DAY: u64 = 24 * HOUR;
const EXPIRIES: [(u64, &str); 3] = [(HOUR, "1 hour"), (DAY, "1 day"), (7 * DAY, "7 days")];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// A link served by this Scratchpad (`create_share` / `create_mailbox_share`).
    Local,
    /// Upload to the S3 config with this id.
    S3(String),
}

/// The share dialog.
#[derive(Debug, Clone)]
pub struct ShareDialog {
    pub title: String,
    pub ids: Vec<String>,
    /// Sharing a whole mailbox: the server link stays live.
    pub mailbox: Option<String>,
    pub destination: Destination,
    pub visibility: Visibility,
    pub expiry: u64,
    pub attachments: bool,
    pub technical: bool,
    pub busy: bool,
}

/// Cloud sharing state shown on the Cloud sharing and Shares pages.
pub struct Cloud {
    pub configs: Vec<S3Config>,
    pub vault: VaultStatus,
    pub shares: Vec<CloudShare>,
    pub form: Option<S3Form>,
    pub saving: bool,
    /// Config id (or [`FORM`]) being tested.
    pub testing: Option<String>,
    pub checks: Option<(String, Vec<S3Check>)>,
    pub vault_form: Option<VaultForm>,
    pub vault_busy: bool,
    /// Cloud share being deleted.
    pub deleting: Option<String>,
}

impl Cloud {
    pub fn load(scratchpad: &scratchpad_core::Scratchpad) -> Self {
        Self {
            configs: scratchpad.s3_configs(),
            vault: scratchpad.vault_status(),
            shares: scratchpad.cloud_shares().unwrap_or_default(),
            form: None,
            saving: false,
            testing: None,
            checks: None,
            vault_form: None,
            vault_busy: false,
            deleting: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CloudMsg {
    // Share dialog.
    Title(String),
    Destination(Destination),
    Visibility(Visibility),
    Expiry(u64),
    Attachments(bool),
    Technical(bool),
    Submit,
    Cancel,
    Uploaded(Result<CloudShare, String>),
    SetUp,

    // Cloud shares.
    Renew(String, u64),
    ShareDeleted(String, Result<(), String>),

    // S3 configs.
    NewConfig,
    EditConfig(String),
    Form(S3Msg),
    SaveConfig,
    CancelConfig,
    TestForm,
    TestConfig(String),
    Tested(String, Vec<S3Check>),
    ConfigSaved(Result<S3Config, String>),
    ConfigDeleted(Result<(), String>),

    // Credential vault.
    VaultOpen(VaultAction),
    VaultInput(VaultField, String),
    VaultSubmit,
    VaultCancel,
    VaultLock,
    VaultSync,
    VaultDone(Result<String, String>),
}

fn cloud(msg: CloudMsg) -> Message {
    Message::Cloud(msg)
}

fn mode_label(mode: S3Mode) -> &'static str {
    match mode {
        S3Mode::Public => "Public",
        S3Mode::Private => "Private",
        S3Mode::Both => "Public & private",
    }
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// "Expires in 5h" / "Expired", for private links.
fn expiry_chip<'a>(expires_at: i64) -> Element<'a, Message> {
    let left = expires_at - chrono::Utc::now().timestamp();
    if left <= 0 {
        return chip_icon(Icon::Clock, "Expired", Tone::Bad);
    }
    let label = match left {
        l if l < HOUR as i64 => format!("Expires in {}m", (l / 60).max(1)),
        l if l < 2 * DAY as i64 => format!("Expires in {}h", l / HOUR as i64),
        l => format!("Expires in {}d", l / DAY as i64),
    };
    let tone = if left < HOUR as i64 {
        Tone::Warn
    } else {
        Tone::Neutral
    };
    chip_icon(Icon::Clock, label, tone)
}

fn expiry_text(secs: u64) -> &'static str {
    EXPIRIES
        .iter()
        .find(|(s, _)| *s == secs)
        .map(|(_, label)| *label)
        .unwrap_or("1 day")
}

// ---------------------------------------------------------------- update

impl App {
    pub(super) fn reload_cloud(&mut self) {
        self.cloud.configs = self.scratchpad.s3_configs();
        self.cloud.vault = self.scratchpad.vault_status();
        self.cloud.shares = self.scratchpad.cloud_shares().unwrap_or_default();
        // A synced config disappears when the vault locks.
        if let Some(dialog) = &mut self.share_dialog {
            if let Destination::S3(id) = &dialog.destination {
                if !self.cloud.configs.iter().any(|c| &c.id == id) {
                    dialog.destination = default_destination(&self.cloud.configs);
                }
            }
        }
    }

    /// Opens the share dialog for `ids` (or a whole mailbox).
    pub(super) fn open_share_dialog(
        &mut self,
        title: String,
        ids: Vec<String>,
        mailbox: Option<String>,
    ) -> Task<Message> {
        if ids.is_empty() && mailbox.is_none() {
            return Task::none();
        }
        self.reload_cloud();
        let destination = default_destination(&self.cloud.configs);
        let mut dialog = ShareDialog {
            title,
            ids,
            mailbox,
            destination,
            visibility: Visibility::Public,
            expiry: DEFAULT_EXPIRY_SECS,
            attachments: true,
            technical: true,
            busy: false,
        };
        dialog.visibility = self
            .dialog_config(&dialog)
            .map_or(Visibility::Public, |c| c.mode.default_visibility());
        self.share_dialog = Some(dialog);
        iced::widget::operation::focus(SHARE_TITLE)
    }

    fn dialog_config(&self, dialog: &ShareDialog) -> Option<&S3Config> {
        match &dialog.destination {
            Destination::S3(id) => self.cloud.configs.iter().find(|c| &c.id == id),
            Destination::Local => None,
        }
    }

    pub(super) fn close_share_dialog(&mut self) {
        if !self.share_dialog.as_ref().is_some_and(|d| d.busy) {
            self.share_dialog = None;
        }
    }

    /// Runs `future` on the tokio runtime and maps its result to a message.
    fn spawn<T: Send + 'static>(
        &self,
        future: impl std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
        message: impl FnOnce(Result<T, String>) -> Message + Send + 'static,
    ) -> Task<Message> {
        let task = self
            .runtime
            .spawn(async move { future.await.map_err(|e| format!("{e:#}")) });
        Task::perform(
            async move { task.await.unwrap_or_else(|e| Err(e.to_string())) },
            message,
        )
    }

    /// Confirmed actions that belong to cloud sharing.
    pub(super) fn cloud_confirmed(&mut self, confirm: Confirm) -> Task<Message> {
        let scratchpad = self.scratchpad.clone();
        let keep_object = matches!(confirm, Confirm::ForgetCloudShare(..));
        match confirm {
            Confirm::DeleteCloudShare(id) | Confirm::ForgetCloudShare(id, _) => {
                self.cloud.deleting = Some(id.clone());
                let target = id.clone();
                self.spawn(
                    async move { scratchpad.delete_cloud_share(&target, keep_object).await },
                    move |result| cloud(CloudMsg::ShareDeleted(id, result)),
                )
            }
            Confirm::DeleteS3Config(id) => {
                if self.cloud.form.as_ref().is_some_and(|f| f.id == id) {
                    self.cloud.form = None;
                }
                self.spawn(async move { scratchpad.delete_s3_config(&id).await }, |r| {
                    cloud(CloudMsg::ConfigDeleted(r))
                })
            }
            Confirm::ResetVault => {
                self.cloud.vault_busy = true;
                self.cloud.vault_form = None;
                self.spawn(
                    async move {
                        scratchpad.vault_reset().await?;
                        Ok("Vault reset. Synced buckets were removed.".to_string())
                    },
                    |r| cloud(CloudMsg::VaultDone(r)),
                )
            }
            _ => Task::none(),
        }
    }

    pub(super) fn update_cloud(&mut self, msg: CloudMsg) -> Task<Message> {
        match msg {
            CloudMsg::Title(value) => {
                if let Some(d) = &mut self.share_dialog {
                    d.title = value;
                }
            }
            CloudMsg::Destination(destination) => {
                if let Some(mut dialog) = self.share_dialog.take() {
                    dialog.destination = destination;
                    if let Some(config) = self.dialog_config(&dialog) {
                        if !config.mode.allows(dialog.visibility) {
                            dialog.visibility = config.mode.default_visibility();
                        }
                    }
                    self.share_dialog = Some(dialog);
                }
            }
            CloudMsg::Visibility(visibility) => {
                if let Some(d) = &mut self.share_dialog {
                    d.visibility = visibility;
                }
            }
            CloudMsg::Expiry(secs) => {
                if let Some(d) = &mut self.share_dialog {
                    d.expiry = secs.min(MAX_PRESIGN_SECS);
                }
            }
            CloudMsg::Attachments(value) => {
                if let Some(d) = &mut self.share_dialog {
                    d.attachments = value;
                }
            }
            CloudMsg::Technical(value) => {
                if let Some(d) = &mut self.share_dialog {
                    d.technical = value;
                }
            }
            CloudMsg::Cancel => self.close_share_dialog(),
            CloudMsg::SetUp => {
                self.close_share_dialog();
                return self.update(Message::Page(Page::Cloud));
            }
            CloudMsg::Submit => return self.submit_share(),
            CloudMsg::Uploaded(result) => match result {
                Ok(share) => {
                    self.share_dialog = None;
                    self.reload_cloud();
                    self.toast("Uploaded. Link copied to clipboard");
                    return iced::clipboard::write(share.url);
                }
                Err(e) => {
                    if let Some(d) = &mut self.share_dialog {
                        d.busy = false;
                    }
                    self.error(format!("Upload failed: {e}"));
                }
            },

            CloudMsg::Renew(id, secs) => match self.scratchpad.renew_cloud_share(&id, Some(secs)) {
                Ok(share) => {
                    self.reload_cloud();
                    self.toast(format!(
                        "New link copied to clipboard, valid for {}",
                        expiry_text(secs)
                    ));
                    return iced::clipboard::write(share.url);
                }
                Err(e) => self.error(format!("Could not renew the link: {e:#}")),
            },
            CloudMsg::ShareDeleted(id, result) => {
                self.cloud.deleting = None;
                self.reload_cloud();
                match result {
                    Ok(()) => self.toast("Share deleted"),
                    Err(e) => {
                        let gone = !self.cloud.shares.iter().any(|s| s.id == id);
                        if gone {
                            self.error(e);
                        } else {
                            // Usually offline or the bucket is gone: offer to just forget it.
                            self.error(format!(
                                "Couldn't delete the uploaded file: {e}. You can remove it from the list instead."
                            ));
                            self.confirm = Some(Confirm::ForgetCloudShare(id, e));
                        }
                    }
                }
            }

            CloudMsg::NewConfig => {
                self.cloud.form = Some(S3Form::default());
                self.cloud.checks = None;
            }
            CloudMsg::EditConfig(id) => {
                self.cloud.form = self
                    .cloud
                    .configs
                    .iter()
                    .find(|c| c.id == id)
                    .map(S3Form::from_config);
                self.cloud.checks = None;
            }
            CloudMsg::Form(msg) => {
                if let Some(form) = &mut self.cloud.form {
                    form.update(msg);
                }
            }
            CloudMsg::CancelConfig => {
                self.cloud.form = None;
                if self.cloud.checks.as_ref().is_some_and(|(k, _)| k == FORM) {
                    self.cloud.checks = None;
                }
            }
            CloudMsg::SaveConfig => {
                let Some(form) = &self.cloud.form else {
                    return Task::none();
                };
                if self.cloud.saving {
                    return Task::none();
                }
                if let Err(e) = form.validate(false) {
                    self.error(e);
                    return Task::none();
                }
                let config = form.build();
                self.cloud.saving = true;
                let scratchpad = self.scratchpad.clone();
                return self.spawn(
                    async move { scratchpad.save_s3_config(config).await },
                    |r| cloud(CloudMsg::ConfigSaved(r)),
                );
            }
            CloudMsg::ConfigSaved(result) => {
                self.cloud.saving = false;
                match result {
                    Ok(config) => {
                        self.cloud.form = None;
                        self.cloud.checks = None;
                        self.reload_cloud();
                        self.toast(format!("Saved “{}”", config.name));
                    }
                    Err(e) => self.error(e),
                }
            }
            CloudMsg::TestForm => {
                if let Some(form) = &self.cloud.form {
                    if self.cloud.testing.is_some() {
                        return Task::none();
                    }
                    if let Err(e) = form.validate(true) {
                        self.error(e);
                        return Task::none();
                    }
                    let config = form.build();
                    return self.test(FORM.to_string(), config);
                }
            }
            CloudMsg::TestConfig(id) => {
                if self.cloud.testing.is_some() {
                    return Task::none();
                }
                if let Some(config) = self.cloud.configs.iter().find(|c| c.id == id).cloned() {
                    return self.test(id, config);
                }
            }
            CloudMsg::Tested(key, checks) => {
                self.cloud.testing = None;
                let failed = checks.iter().filter(|c| !c.ok).count();
                if failed == 0 {
                    self.toast("All checks passed");
                } else {
                    self.error(format!("{} failed", plural(failed, "check")));
                }
                self.cloud.checks = Some((key, checks));
            }
            CloudMsg::ConfigDeleted(result) => {
                self.reload_cloud();
                match result {
                    Ok(()) => self.toast("Bucket removed"),
                    Err(e) => self.error(e),
                }
            }

            CloudMsg::VaultOpen(action) => self.cloud.vault_form = Some(VaultForm::new(action)),
            CloudMsg::VaultInput(field, value) => {
                if let Some(form) = &mut self.cloud.vault_form {
                    form.set(field, value);
                }
            }
            CloudMsg::VaultCancel => self.cloud.vault_form = None,
            CloudMsg::VaultSubmit => return self.submit_vault(),
            CloudMsg::VaultLock => {
                self.scratchpad.vault_lock();
                if self
                    .cloud
                    .form
                    .as_ref()
                    .is_some_and(|f| f.storage == CredentialStorage::Synced && !f.is_new())
                {
                    self.cloud.form = None;
                }
                self.reload_cloud();
                self.toast("Vault locked");
            }
            CloudMsg::VaultSync => {
                self.cloud.vault_busy = true;
                let scratchpad = self.scratchpad.clone();
                return self.spawn(
                    async move {
                        scratchpad.vault_sync_now().await?;
                        Ok("Vault synced".to_string())
                    },
                    |r| cloud(CloudMsg::VaultDone(r)),
                );
            }
            CloudMsg::VaultDone(result) => {
                self.cloud.vault_busy = false;
                self.reload_cloud();
                match result {
                    Ok(message) => {
                        self.cloud.vault_form = None;
                        self.toast(message);
                    }
                    Err(e) => self.error(e),
                }
            }
        }
        Task::none()
    }

    fn test(&mut self, key: String, config: S3Config) -> Task<Message> {
        self.cloud.testing = Some(key.clone());
        let scratchpad = self.scratchpad.clone();
        self.spawn(
            async move { Ok(scratchpad.test_s3_config(config).await) },
            move |result| match result {
                Ok(checks) => cloud(CloudMsg::Tested(key, checks)),
                Err(e) => cloud(CloudMsg::Tested(
                    key,
                    vec![S3Check {
                        name: "Test".into(),
                        ok: false,
                        detail: e,
                    }],
                )),
            },
        )
    }

    fn submit_share(&mut self) -> Task<Message> {
        let Some(dialog) = self.share_dialog.clone() else {
            return Task::none();
        };
        if dialog.busy {
            return Task::none();
        }
        let title = match dialog.title.trim() {
            "" => "Shared emails".to_string(),
            title => title.to_string(),
        };
        match dialog.destination {
            Destination::Local => {
                let result = match &dialog.mailbox {
                    Some(mailbox) => self.scratchpad.create_mailbox_share(&title, mailbox),
                    None => self.scratchpad.create_share(&title, dialog.ids.clone()),
                };
                if result.is_ok() {
                    self.share_dialog = None;
                }
                self.shared(result)
            }
            Destination::S3(config_id) => {
                if let Some(d) = &mut self.share_dialog {
                    d.busy = true;
                }
                let request = CloudShareRequest {
                    title,
                    ids: dialog.ids,
                    config_id,
                    visibility: Some(dialog.visibility),
                    expires_in_secs: (dialog.visibility == Visibility::Private)
                        .then_some(dialog.expiry),
                    options: ExportOptions {
                        attachments: dialog.attachments,
                        technical: dialog.technical,
                        device: None,
                    },
                };
                let scratchpad = self.scratchpad.clone();
                self.spawn(
                    async move { scratchpad_web::export::share_to_cloud(&scratchpad, request).await },
                    |r| cloud(CloudMsg::Uploaded(r)),
                )
            }
        }
    }

    fn submit_vault(&mut self) -> Task<Message> {
        let Some(form) = self.cloud.vault_form.clone() else {
            return Task::none();
        };
        if self.cloud.vault_busy {
            return Task::none();
        }
        if let Err(e) = form.validate() {
            self.error(e);
            return Task::none();
        }
        self.cloud.vault_busy = true;
        let scratchpad = self.scratchpad.clone();
        let VaultForm {
            action,
            password,
            new_password,
            ..
        } = form;
        self.spawn(
            async move {
                Ok(match action {
                    VaultAction::Create => {
                        scratchpad.vault_create(password).await?;
                        "Master password set. The vault is unlocked.".to_string()
                    }
                    VaultAction::Unlock => {
                        scratchpad.vault_unlock(password).await?;
                        "Vault unlocked".to_string()
                    }
                    VaultAction::ChangePassword => {
                        scratchpad
                            .vault_change_password(password, new_password)
                            .await?;
                        "Master password changed".to_string()
                    }
                    VaultAction::Restore => {
                        let count = scratchpad.vault_restore(password).await?;
                        format!("Restored {} from the cloud", plural(count, "synced bucket"))
                    }
                })
            },
            |r| cloud(CloudMsg::VaultDone(r)),
        )
    }
}

fn default_destination(configs: &[S3Config]) -> Destination {
    configs
        .first()
        .map_or(Destination::Local, |c| Destination::S3(c.id.clone()))
}

// ---------------------------------------------------------------- shared widgets

/// A segmented control.
fn segmented<'a, T: Copy + PartialEq + 'a>(
    options: &[(T, &'a str)],
    selected: T,
    on_select: impl Fn(T) -> Option<Message>,
) -> Element<'a, Message> {
    container(
        Row::with_children(options.iter().map(|(value, label)| {
            button(text(*label).size(12.5).font(MEDIUM))
                .padding([4, 10])
                .style(style::btn(Kind::Segment(*value == selected)))
                .on_press_maybe(on_select(*value))
                .into()
        }))
        .spacing(2),
    )
    .padding(2)
    .style(style::segmented)
    .into()
}

fn field<'a>(
    placeholder: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> Message + 'a,
) -> text_input::TextInput<'a, Message> {
    text_input(placeholder, value)
        .on_input(on_input)
        .padding([7, 10])
        .size(13)
        .style(style::field)
}

fn switch<'a>(value: bool, on_toggle: impl Fn(bool) -> Message + 'a) -> Element<'a, Message> {
    toggler(value)
        .size(18)
        .style(style::switch)
        .on_toggle(on_toggle)
        .into()
}

/// A settings row whose control gets the full width, below the label.
fn stacked<'a>(
    label: &'a str,
    help: Option<&'a str>,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut content = column![tx(label, 13.0, fg).font(MEDIUM)].spacing(2);
    if let Some(help) = help {
        content = content.push(muted(help));
    }
    content
        .push(space::vertical().height(6))
        .push(control)
        .padding([12, 18])
        .into()
}

fn check_rows<'a>(checks: &'a [S3Check]) -> Element<'a, Message> {
    Column::with_children(checks.iter().map(|check| {
        row![
            icon(
                if check.ok {
                    Icon::CircleCheck
                } else {
                    Icon::CircleX
                },
                15.0,
                if check.ok { |t| t.ok } else { danger_c },
            ),
            tx(check.name.clone(), 12.5, fg)
                .font(MEDIUM)
                .width(Length::Fixed(170.0)),
            tx(check.detail.clone(), 12.0, muted_c)
                .font(MONO)
                .width(Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Start)
        .into()
    }))
    .spacing(8)
    .into()
}

// ---------------------------------------------------------------- share dialog

/// The share dialog over a dimmed backdrop that swallows clicks.
pub fn share_dialog<'a>(app: &'a App, d: &'a ShareDialog) -> Element<'a, Message> {
    let count = d.ids.len();
    let what = match &d.mailbox {
        Some(mailbox) => format!("Mailbox “{mailbox}” · {}", plural(count, "email")),
        None => plural(count, "email"),
    };
    let head = row![
        tile_icon(Icon::Share2),
        column![tx("Share", 16.0, fg).font(SEMIBOLD), muted(what)]
            .spacing(1)
            .width(Fill),
        icon_btn(Icon::X, cloud(CloudMsg::Cancel), false),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let label = |title: &'static str| tx(title, 12.5, fg).font(MEDIUM);

    let option = |selected: bool,
                  glyph: Icon,
                  title: String,
                  detail: String,
                  destination: Destination|
     -> Element<'a, Message> {
        button(
            row![
                icon(
                    glyph,
                    16.0,
                    if selected {
                        super::view::accent_c
                    } else {
                        muted_c
                    }
                ),
                column![tx(title, 13.0, fg).font(MEDIUM), muted(detail)]
                    .spacing(1)
                    .width(Fill),
                if selected {
                    icon(Icon::Check, 15.0, super::view::accent_c)
                } else {
                    space::horizontal().width(15).into()
                },
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .width(Fill)
        .padding([8, 10])
        .style(move |theme: &Theme, status| {
            let mut style = style::row(selected)(theme, status);
            let t = style::tokens(theme);
            style.border = iced::Border {
                color: if selected { t.accent } else { t.border },
                width: 1.0,
                radius: 8.0.into(),
            };
            style
        })
        .on_press_maybe((!d.busy).then(|| cloud(CloudMsg::Destination(destination))))
        .into()
    };

    let mut destinations = Column::new().spacing(6);
    for config in &app.cloud.configs {
        let selected = d.destination == Destination::S3(config.id.clone());
        destinations = destinations.push(option(
            selected,
            if config.storage == CredentialStorage::Synced {
                Icon::Cloud
            } else {
                Icon::CloudUpload
            },
            config.name.clone(),
            format!(
                "{} links · {} / {}",
                mode_label(config.mode),
                config
                    .endpoint
                    .trim_start_matches("https://")
                    .trim_start_matches("http://"),
                config.bucket
            ),
            Destination::S3(config.id.clone()),
        ));
    }
    let local_detail = match (&d.mailbox, app.scratchpad.web_url().is_some()) {
        (_, false) => "Needs the web interface (Settings → Web interface)".to_string(),
        (Some(_), true) => "Live: new mail in this mailbox shows up automatically".to_string(),
        (None, true) => "Works while this app is running and reachable".to_string(),
    };
    destinations = destinations.push(option(
        d.destination == Destination::Local,
        Icon::Server,
        "Live link from this Scratchpad server".into(),
        local_detail,
        Destination::Local,
    ));

    let mut body = column![
        column![
            label("Title"),
            field("Shared emails", &d.title, |v| cloud(CloudMsg::Title(v)))
                .id(SHARE_TITLE)
                .on_submit(cloud(CloudMsg::Submit)),
        ]
        .spacing(6),
        column![label("Destination"), destinations].spacing(6),
    ]
    .spacing(16);

    if app.cloud.configs.is_empty() {
        body = body.push(
            container(
                row![
                    icon(Icon::Cloud, 15.0, muted_c),
                    muted("Upload to your own S3 bucket for links that work without this app.")
                        .width(Fill),
                    btn(
                        None,
                        "Set up cloud sharing",
                        Kind::Ghost,
                        Some(cloud(CloudMsg::SetUp))
                    ),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .padding([8, 12])
            .style(style::code_block),
        );
    }

    let primary = match app.dialog_config(d) {
        Some(config) => {
            let allowed: Vec<(Visibility, &str)> = [
                (Visibility::Public, "Public"),
                (Visibility::Private, "Private"),
            ]
            .into_iter()
            .filter(|(v, _)| config.mode.allows(*v))
            .collect();
            if allowed.len() > 1 {
                body = body.push(
                    row![
                        column![
                            label("Visibility"),
                            muted(match d.visibility {
                                Visibility::Public => "Anyone with the link, until you delete it",
                                Visibility::Private => "A presigned link that expires",
                            })
                        ]
                        .spacing(2)
                        .width(Fill),
                        segmented(&allowed, d.visibility, |v| {
                            Some(cloud(CloudMsg::Visibility(v)))
                        }),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                );
            }
            if d.visibility == Visibility::Private {
                body = body.push(
                    row![
                        column![
                            label("Link expires after"),
                            muted("Renew it from the Shares page.")
                        ]
                        .spacing(2)
                        .width(Fill),
                        segmented(&EXPIRIES.map(|(s, l)| (s, l)), d.expiry, |s| {
                            Some(cloud(CloudMsg::Expiry(s)))
                        }),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                );
            }
            let toggle = |title: &'static str, value: bool, f: fn(bool) -> CloudMsg| {
                row![
                    tx(title, 12.5, fg).width(Fill),
                    switch(value, move |v| cloud(f(v)))
                ]
                .spacing(12)
                .align_y(Alignment::Center)
            };
            body = body.push(
                column![
                    toggle("Include attachments", d.attachments, CloudMsg::Attachments),
                    toggle(
                        "Include technical details (headers, raw source, Bcc, spam & link reports)",
                        d.technical,
                        CloudMsg::Technical
                    ),
                ]
                .spacing(10),
            );
            if d.busy {
                "Uploading…"
            } else {
                "Upload & copy link"
            }
        }
        None => "Create link",
    };

    let footer = row![
        space::horizontal(),
        btn(
            None,
            "Cancel",
            Kind::Secondary,
            (!d.busy).then_some(cloud(CloudMsg::Cancel))
        ),
        btn(
            Some(if d.destination == Destination::Local {
                Icon::Link
            } else {
                Icon::CloudUpload
            }),
            primary,
            Kind::Primary,
            (!d.busy).then_some(cloud(CloudMsg::Submit))
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let card = container(
        column![
            container(head).padding([16, 20]),
            hairline(),
            // Scrolls when the window is too short for the whole dialog.
            container(scrollable(container(body).padding([18, 20])).style(style::scroll))
                .max_height((app.window.height - 190.0).max(120.0)),
            hairline(),
            container(footer).padding([12, 20]),
        ]
        .width(Fill),
    )
    .max_width(520)
    .style(style::dialog);

    opaque(
        container(card)
            .center(Fill)
            .padding(24)
            .style(style::backdrop),
    )
}

// ---------------------------------------------------------------- shares page

/// Cards for the uploaded shares.
pub fn cloud_share_cards(app: &App) -> Vec<Element<'_, Message>> {
    app.cloud
        .shares
        .iter()
        .map(|share| {
            let private = share.visibility == Visibility::Private;
            let mut title = row![
                tx(share.title.clone(), 13.0, fg).font(SEMIBOLD),
                if private {
                    chip_icon(Icon::Lock, "Private", Tone::Neutral)
                } else {
                    chip_icon(Icon::Globe, "Public", Tone::Ok)
                },
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            if let Some(expires_at) = share.expires_at {
                title = title.push(expiry_chip(expires_at));
            }
            let summary = format!(
                "{} · {} · {} · {}",
                plural(share.emails.len(), "email"),
                share.config_name,
                format_size(share.size),
                format_time(share.created_at)
            );
            let deleting = app.cloud.deleting.as_deref() == Some(share.id.as_str());

            let mut actions = Row::new().spacing(2).align_y(Alignment::Center);
            if private {
                let renew = |secs: u64, label: &'static str| {
                    button(text(label).size(12).font(MEDIUM))
                        .padding([3, 8])
                        .style(style::btn(Kind::Segment(false)))
                        .on_press(cloud(CloudMsg::Renew(share.id.clone(), secs)))
                };
                actions = actions.push(
                    container(
                        row![
                            container(
                                row![icon(Icon::RefreshCw, 12.0, muted_c), faint("Renew")]
                                    .spacing(4)
                                    .align_y(Alignment::Center)
                            )
                            .padding([0, 6]),
                            renew(DAY, "1 day"),
                            renew(7 * DAY, "7 days"),
                        ]
                        .spacing(2)
                        .align_y(Alignment::Center),
                    )
                    .padding(2)
                    .style(style::segmented),
                );
                actions = actions.push(space::horizontal().width(6));
            }
            actions = actions
                .push(icon_btn(
                    Icon::Copy,
                    Message::Copy(share.url.clone()),
                    false,
                ))
                .push(icon_btn(
                    Icon::ExternalLink,
                    Message::OpenUrl(share.url.clone()),
                    false,
                ));
            let delete: Element<'_, Message> = if deleting {
                container(faint("Deleting…")).padding([0, 6]).into()
            } else {
                icon_btn(
                    Icon::Trash2,
                    Message::Ask(Confirm::DeleteCloudShare(share.id.clone())),
                    true,
                )
                .into()
            };
            actions = actions.push(delete);

            container(
                row![
                    tile_icon(if private { Icon::Lock } else { Icon::Cloud }),
                    column![title, muted(summary)].spacing(3).width(Fill),
                    actions,
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding([12, 16])
            .style(style::card)
            .into()
        })
        .collect()
}

// ---------------------------------------------------------------- cloud page

pub fn cloud_page(app: &App) -> Element<'_, Message> {
    let c = &app.cloud;
    let mut content = column![page_head(
        "Cloud sharing",
        "Upload emails as one self-contained .html file to your own S3-compatible bucket (AWS S3, Cloudflare R2, MinIO, Backblaze B2, …) and share a public or expiring private link."
    )]
    .spacing(12);

    if let Some(form) = &c.form {
        content = content.push(s3_form(app, form));
    }

    if c.configs.is_empty() && c.form.is_none() {
        content = content.push(
            container(empty(
                Icon::CloudUpload,
                "No buckets yet",
                "Add an S3-compatible bucket to upload shares that work without this app.",
                None,
            ))
            .height(260),
        );
    }
    for config in &c.configs {
        content = content.push(config_card(app, config));
    }
    if c.vault.exists && !c.vault.unlocked {
        content = content.push(
            row![
                icon(Icon::Lock, 13.0, muted_c),
                muted("Synced buckets are hidden until you unlock the vault.")
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }
    if c.form.is_none() {
        content = content.push(btn(
            Some(Icon::Plus),
            "Add bucket",
            Kind::Primary,
            Some(cloud(CloudMsg::NewConfig)),
        ));
    }
    content = content.push(space::vertical().height(8)).push(vault(app));
    content.into()
}

fn config_card<'a>(app: &'a App, config: &'a S3Config) -> Element<'a, Message> {
    let synced = config.storage == CredentialStorage::Synced;
    let title = row![
        tx(config.name.clone(), 13.0, fg).font(SEMIBOLD),
        chip(mode_label(config.mode), Tone::Neutral),
        if synced {
            chip_icon(Icon::Cloud, "Synced", Tone::Neutral)
        } else {
            chip_icon(Icon::HardDrive, "Local", Tone::Neutral)
        },
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let mut where_ = format!("{} / {}", config.endpoint, config.bucket);
    if !config.cdn_domain.is_empty() {
        where_.push_str(&format!(" · {}", config.cdn_domain));
    }
    let testing = app.cloud.testing.as_deref() == Some(config.id.as_str());
    let mut card = column![row![
        tile_icon(if synced { Icon::Cloud } else { Icon::HardDrive }),
        column![title, muted(where_)].spacing(3).width(Fill),
        btn(
            None,
            if testing { "Testing…" } else { "Test" },
            Kind::Secondary,
            (app.cloud.testing.is_none()).then(|| cloud(CloudMsg::TestConfig(config.id.clone())))
        ),
        btn(
            None,
            "Edit",
            Kind::Secondary,
            Some(cloud(CloudMsg::EditConfig(config.id.clone())))
        ),
        icon_btn(
            Icon::Trash2,
            Message::Ask(Confirm::DeleteS3Config(config.id.clone())),
            true
        ),
    ]
    .spacing(12)
    .align_y(Alignment::Center)]
    .spacing(12);
    if let Some((key, checks)) = &app.cloud.checks {
        if key == &config.id {
            card = card.push(
                container(check_rows(checks))
                    .padding([10, 14])
                    .width(Fill)
                    .style(style::code_block),
            );
        }
    }
    container(card).padding([12, 16]).style(style::card).into()
}

fn s3_form<'a>(app: &'a App, form: &'a S3Form) -> Element<'a, Message> {
    let input = move |placeholder: &'a str, value: &'a str, f: S3Field| {
        field(placeholder, value, move |v| {
            cloud(CloudMsg::Form(S3Msg::Text(f, v)))
        })
    };
    let unlocked = app.cloud.vault.unlocked;
    let mode_help = match form.mode {
        S3Mode::Public => "Links are <host>/email/shares/<uuid>.html. Make that prefix publicly readable (bucket policy or CDN).",
        S3Mode::Private => "Shares are presigned links (up to 7 days) to files stored under email/private/.",
        S3Mode::Both => "Pick per share: public links under email/shares/, or expiring presigned links under email/private/.",
    };
    let storage_help = if unlocked {
        "Synced buckets are encrypted with your master password and available on your other devices."
    } else {
        "To sync, set or unlock the master password in Credential vault below."
    };
    let mut rows = vec![
        setting(
            "Name",
            None,
            input("Production R2", &form.name, S3Field::Name),
        ),
        setting(
            "S3 endpoint",
            Some("e.g. https://<account>.r2.cloudflarestorage.com"),
            input(
                "https://s3.eu-west-1.amazonaws.com",
                &form.endpoint,
                S3Field::Endpoint,
            ),
        ),
        setting(
            "Region",
            Some("“auto” works for R2. Use the bucket's region for AWS."),
            input("auto", &form.region, S3Field::Region),
        ),
        setting(
            "Bucket",
            None,
            input("mails", &form.bucket, S3Field::Bucket),
        ),
        setting(
            "Access key",
            None,
            input("", &form.access_key, S3Field::AccessKey),
        ),
        setting(
            "Secret key",
            (!form.is_new()).then_some("Leave empty to keep the stored secret."),
            input(
                if form.is_new() { "" } else { "Unchanged" },
                &form.secret_key,
                S3Field::SecretKey,
            )
            .secure(true),
        ),
        setting(
            "Vanity/CDN domain",
            Some("Optional. Public links use this host instead of the endpoint."),
            input("share.example.com", &form.cdn_domain, S3Field::CdnDomain),
        ),
        stacked(
            "Link mode",
            Some(mode_help),
            segmented(
                &[
                    (S3Mode::Public, "Public"),
                    (S3Mode::Private, "Private"),
                    (S3Mode::Both, "Both"),
                ],
                form.mode,
                |m| Some(cloud(CloudMsg::Form(S3Msg::Mode(m)))),
            ),
        ),
        setting(
            "Path-style URLs",
            Some("Turn off for AWS virtual-hosted buckets."),
            switch(form.path_style, |v| {
                cloud(CloudMsg::Form(S3Msg::PathStyle(v)))
            }),
        ),
        stacked(
            "Credentials",
            Some(storage_help),
            segmented(
                &[
                    (CredentialStorage::Local, "This device only"),
                    (
                        CredentialStorage::Synced,
                        "Synced, encrypted with master password",
                    ),
                ],
                form.storage,
                |s| {
                    (s == CredentialStorage::Local || unlocked)
                        .then(|| cloud(CloudMsg::Form(S3Msg::Storage(s))))
                },
            ),
        ),
    ];
    if let Some((key, checks)) = &app.cloud.checks {
        if key == FORM {
            rows.push(container(check_rows(checks)).padding([12, 18]).into());
        }
    }
    let testing = app.cloud.testing.as_deref() == Some(FORM);
    rows.push(
        row![
            btn(
                Some(Icon::RefreshCw),
                if testing {
                    "Testing…"
                } else {
                    "Test connection"
                },
                Kind::Secondary,
                app.cloud
                    .testing
                    .is_none()
                    .then_some(cloud(CloudMsg::TestForm))
            ),
            space::horizontal(),
            btn(
                None,
                "Cancel",
                Kind::Secondary,
                Some(cloud(CloudMsg::CancelConfig))
            ),
            btn(
                Some(Icon::Check),
                if app.cloud.saving {
                    "Saving…"
                } else {
                    "Save"
                },
                Kind::Primary,
                (!app.cloud.saving).then_some(cloud(CloudMsg::SaveConfig))
            ),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .padding([12, 18])
        .into(),
    );
    section(
        if form.is_new() {
            "Add bucket"
        } else {
            "Edit bucket"
        },
        Some("Test connection uploads, reads and deletes a small file to check the setup."),
        rows,
    )
}

fn vault(app: &App) -> Element<'_, Message> {
    let v = &app.cloud.vault;
    let busy = app.cloud.vault_busy;
    let state = if !v.exists {
        chip_icon(Icon::KeyRound, "No master password", Tone::Neutral)
    } else if !v.unlocked {
        chip_icon(Icon::Lock, "Locked", Tone::Warn)
    } else {
        chip_icon(
            Icon::LockOpen,
            format!(
                "Unlocked · {}",
                plural(v.synced_configs.unwrap_or(0), "synced bucket")
            ),
            Tone::Ok,
        )
    };
    let last_sync: Element<'_, Message> = {
        let mut col = column![muted(
            v.sync
                .last_sync_at
                .map(format_full)
                .unwrap_or_else(|| "Never".into())
        )]
        .align_x(Alignment::End)
        .spacing(2);
        if let Some(error) = &v.sync.last_error {
            col = col.push(tx(error.clone(), 12.0, danger_c));
        }
        col.into()
    };
    let backend_state = if v.backend.connected {
        chip("Connected", Tone::Ok)
    } else {
        chip("Not connected", Tone::Neutral)
    };
    let mut rows = vec![
        setting("Status", None, state),
        setting("Last sync", None, last_sync),
        row![
            column![
                tx(&v.backend.name, 13.0, fg).font(MEDIUM),
                muted(&v.backend.note)
            ]
            .spacing(2)
            .width(Fill),
            backend_state
        ]
        .spacing(16)
        .align_y(Alignment::Center)
        .padding([12, 18])
        .into(),
    ];

    match &app.cloud.vault_form {
        Some(form) => {
            fn input<'a>(
                placeholder: &'a str,
                value: &'a str,
                f: VaultField,
            ) -> text_input::TextInput<'a, Message> {
                field(placeholder, value, move |v| {
                    cloud(CloudMsg::VaultInput(f, v))
                })
                .secure(true)
                .on_submit(cloud(CloudMsg::VaultSubmit))
            }
            let (title, help, submit): (&str, &str, &str) = match form.action {
                VaultAction::Create => (
                    "Set master password",
                    "It can't be recovered. If you forget it, reset the vault and add the synced buckets again.",
                    "Set password",
                ),
                VaultAction::Unlock => ("Unlock vault", "Unlocked until you lock it or quit.", "Unlock"),
                VaultAction::ChangePassword => (
                    "Change master password",
                    "Synced buckets are re-encrypted with the new password.",
                    "Change password",
                ),
                VaultAction::Restore => (
                    "Restore from cloud",
                    "Replaces the synced buckets on this device with the ones in the cloud.",
                    "Restore",
                ),
            };
            let mut fields = column![
                tx(title, 13.0, fg).font(MEDIUM),
                muted(help),
                space::vertical().height(4)
            ]
            .spacing(6);
            let min = format!("At least {MIN_MASTER_PASSWORD} characters");
            fields = match form.action {
                VaultAction::Create => fields
                    .push(input(
                        "Master password",
                        &form.password,
                        VaultField::Password,
                    ))
                    .push(input(
                        "Confirm password",
                        &form.confirm,
                        VaultField::Confirm,
                    ))
                    .push(faint(min)),
                VaultAction::Unlock | VaultAction::Restore => fields.push(input(
                    "Master password",
                    &form.password,
                    VaultField::Password,
                )),
                VaultAction::ChangePassword => fields
                    .push(input(
                        "Current password",
                        &form.password,
                        VaultField::Password,
                    ))
                    .push(input(
                        "New password",
                        &form.new_password,
                        VaultField::NewPassword,
                    ))
                    .push(input(
                        "Confirm new password",
                        &form.confirm,
                        VaultField::Confirm,
                    ))
                    .push(faint(min)),
            };
            fields = fields.push(
                row![
                    space::horizontal(),
                    btn(
                        None,
                        "Cancel",
                        Kind::Secondary,
                        (!busy).then_some(cloud(CloudMsg::VaultCancel))
                    ),
                    btn(
                        Some(Icon::KeyRound),
                        if busy { "Working…" } else { submit },
                        Kind::Primary,
                        (!busy).then_some(cloud(CloudMsg::VaultSubmit))
                    ),
                ]
                .spacing(8),
            );
            rows.push(container(fields.max_width(460)).padding([14, 18]).into());
        }
        None => {
            let action = |glyph: Icon, label: &'static str, kind: Kind, msg: CloudMsg| {
                btn(Some(glyph), label, kind, (!busy).then_some(cloud(msg)))
            };
            let mut actions = Row::new().spacing(8).align_y(Alignment::Center);
            if !v.exists {
                actions = actions
                    .push(action(
                        Icon::KeyRound,
                        "Set master password",
                        Kind::Primary,
                        CloudMsg::VaultOpen(VaultAction::Create),
                    ))
                    .push(action(
                        Icon::CloudDownload,
                        "Restore from cloud",
                        Kind::Secondary,
                        CloudMsg::VaultOpen(VaultAction::Restore),
                    ));
            } else if !v.unlocked {
                actions = actions
                    .push(action(
                        Icon::LockOpen,
                        "Unlock",
                        Kind::Primary,
                        CloudMsg::VaultOpen(VaultAction::Unlock),
                    ))
                    .push(action(
                        Icon::CloudDownload,
                        "Restore from cloud",
                        Kind::Secondary,
                        CloudMsg::VaultOpen(VaultAction::Restore),
                    ));
            } else {
                actions = actions
                    .push(action(
                        Icon::Lock,
                        "Lock",
                        Kind::Secondary,
                        CloudMsg::VaultLock,
                    ))
                    .push(action(
                        Icon::KeyRound,
                        "Change password",
                        Kind::Secondary,
                        CloudMsg::VaultOpen(VaultAction::ChangePassword),
                    ))
                    .push(action(
                        Icon::RefreshCw,
                        if busy { "Syncing…" } else { "Sync now" },
                        Kind::Secondary,
                        CloudMsg::VaultSync,
                    ));
            }
            actions = actions.push(space::horizontal());
            if v.exists {
                actions = actions.push(btn(
                    Some(Icon::Trash2),
                    "Reset vault",
                    Kind::Danger,
                    (!busy).then_some(Message::Ask(Confirm::ResetVault)),
                ));
            }
            rows.push(container(actions).padding([12, 18]).into());
        }
    }

    section(
        "Credential vault",
        Some("Synced buckets are encrypted on this device with your master password (Argon2id + XChaCha20-Poly1305). Only the sealed vault is synced; nobody else can read it."),
        rows,
    )
}
