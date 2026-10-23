//! Scratchpad core: a local SMTP server that captures emails, stores them per
//! mailbox and analyses them (link checker, spam report, view inspector).
//!
//! The [`Scratchpad`] service is shared by the desktop GUI and the web interface.

pub mod cloud;
pub mod events;
pub mod forward;
pub mod links;
pub mod model;
pub mod parse;
pub mod remote;
pub mod s3;
pub mod settings;
pub mod smtp;
pub mod spam;
pub mod store;
pub mod tls;
pub mod vault;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::Context;
use rand::distr::{Alphanumeric, SampleString};
use tokio::sync::{broadcast, Notify};

pub use cloud::PublishRequest;
pub use events::Event;
pub use model::*;
pub use s3::{CredentialStorage, S3Check, S3Config, S3ConfigInfo, S3Mode, Visibility};
pub use settings::{ForwardEncryption, ForwardSettings, Settings, ThemePreference};
use store::{NewEmail, Store};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// macOS bundle identifier (`packaging/macos/Info.plist`).
pub const BUNDLE_ID: &str = "com.scalexy.scratchpad";

/// Default location for settings and the email database.
pub fn default_data_dir() -> PathBuf {
    // On macOS the folder is named after the bundle identifier.
    #[cfg(target_os = "macos")]
    let dirs = directories::ProjectDirs::from_path(PathBuf::from(BUNDLE_ID));
    #[cfg(not(target_os = "macos"))]
    let dirs = directories::ProjectDirs::from("com", "ScaleXY", "Scratchpad");
    dirs.map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".scratchpad"))
}

#[derive(Debug, Clone, Default)]
struct SmtpState {
    running: bool,
    address: String,
    error: Option<String>,
}

pub struct Scratchpad {
    data_dir: PathBuf,
    pub(crate) store: Store,
    settings: RwLock<Settings>,
    /// Settings fields changed by `customize` in [`Scratchpad::open`] (CLI flags): their saved
    /// values, so saving from a UI doesn't persist a one-off override.
    overrides: Vec<(String, serde_json::Value, serde_json::Value)>,
    events: broadcast::Sender<Event>,
    pub(crate) http: reqwest::Client,
    /// For S3 uploads: no redirects (presigned URLs are host-bound), generous timeout.
    s3_http: reqwest::Client,
    vault: vault::Vault,
    smtp_state: RwLock<SmtpState>,
    smtp_restart: Notify,
    remote_wakeup: Notify,
    retention_wakeup: Notify,
    web_url: RwLock<Option<String>>,
    started: AtomicBool,
    /// Scratch data directory of [`Scratchpad::in_memory`], removed on drop.
    temp_dir: Option<PathBuf>,
}

impl Drop for Scratchpad {
    fn drop(&mut self) {
        if let Some(dir) = &self.temp_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// How long one poll of a remote mailbox may take. Messages imported before the
/// deadline are kept; the next poll continues where this one stopped.
const REMOTE_POLL_TIMEOUT: Duration = Duration::from_secs(300);

impl Scratchpad {
    /// Opens (or creates) the data directory. `customize` can override persisted settings
    /// (e.g. from CLI flags) without saving them.
    pub fn open(
        data_dir: impl AsRef<Path>,
        customize: impl FnOnce(&mut Settings),
    ) -> anyhow::Result<Arc<Self>> {
        tls::install_crypto_provider();
        let data_dir = data_dir.as_ref().to_path_buf();
        create_private_dir(&data_dir)
            .with_context(|| format!("creating data directory {}", data_dir.display()))?;
        let saved = Settings::load_or_recover(&data_dir);
        let mut settings = saved.clone();
        customize(&mut settings);
        let overrides = setting_overrides(&saved, &settings);
        let store = Store::open(&data_dir.join("scratchpad.sqlite3"))?;
        Self::with_store(data_dir, store, settings, false, overrides)
    }

    /// An in-memory instance, used by tests. Files it writes (vault, credentials) go
    /// to a temporary directory that is removed when the instance is dropped.
    pub fn in_memory(settings: Settings) -> anyhow::Result<Arc<Self>> {
        tls::install_crypto_provider();
        let dir = std::env::temp_dir().join(format!("scratchpad-{}", uuid::Uuid::new_v4()));
        Self::with_store(dir, Store::open_in_memory()?, settings, true, Vec::new())
    }

    fn with_store(
        data_dir: PathBuf,
        store: Store,
        settings: Settings,
        temporary: bool,
        overrides: Vec<(String, serde_json::Value, serde_json::Value)>,
    ) -> anyhow::Result<Arc<Self>> {
        let (events, _) = broadcast::channel(1024);
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "Scratchpad/",
                env!("CARGO_PKG_VERSION"),
                " link-checker"
            ))
            .redirect(reqwest::redirect::Policy::limited(10))
            .tls_backend_preconfigured((*tls::client_config()).clone())
            .build()?;
        let s3_http = reqwest::Client::builder()
            .user_agent(concat!("Scratchpad/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .tls_backend_preconfigured((*tls::client_config()).clone())
            .build()?;
        let vault = vault::Vault::open(
            &data_dir,
            Box::new(vault::StubSyncBackend::new(
                data_dir.join("cloud-sync-stub"),
            )),
            vault::KdfCost::default(),
        )?;
        Ok(Arc::new(Self {
            store,
            settings: RwLock::new(settings),
            overrides,
            events,
            http,
            s3_http,
            vault,
            smtp_state: RwLock::new(SmtpState::default()),
            smtp_restart: Notify::new(),
            remote_wakeup: Notify::new(),
            retention_wakeup: Notify::new(),
            web_url: RwLock::new(None),
            started: AtomicBool::new(false),
            temp_dir: temporary.then(|| data_dir.clone()),
            data_dir,
        }))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    // ---- lifecycle ---------------------------------------------------------

    /// Starts background services (SMTP server, remote poller, retention). Requires a
    /// Tokio runtime. Calling it again has no effect.
    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        tokio::spawn(self.clone().smtp_supervisor());
        tokio::spawn(self.clone().remote_poller());
        tokio::spawn(self.clone().retention_task());
    }

    async fn smtp_supervisor(self: Arc<Self>) {
        loop {
            let settings = self.settings();
            let address = settings.smtp_address();
            let tls = if settings.smtp_starttls {
                match tls::self_signed_acceptor() {
                    Ok(acceptor) => Some(acceptor),
                    Err(error) => {
                        tracing::error!(%error, "could not create TLS certificate, STARTTLS disabled");
                        None
                    }
                }
            } else {
                None
            };
            match tokio::net::TcpListener::bind(&address).await {
                Ok(listener) => {
                    tracing::info!("SMTP server listening on {address}");
                    self.set_smtp_state(true, address, None);
                    tokio::select! {
                        _ = smtp::accept_loop(listener, self.clone(), tls) => {}
                        _ = self.smtp_restart.notified() => {
                            tracing::info!("restarting SMTP server");
                        }
                    }
                }
                Err(error) => {
                    let message = format!("Could not listen on {address}: {error}");
                    tracing::error!("{message}");
                    self.set_smtp_state(false, address, Some(message));
                    tokio::select! {
                        _ = self.smtp_restart.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(10)) => {}
                    }
                }
            }
        }
    }

    fn set_smtp_state(&self, running: bool, address: String, error: Option<String>) {
        *self.smtp_state.write().unwrap() = SmtpState {
            running,
            address: address.clone(),
            error: error.clone(),
        };
        self.emit(Event::SmtpStatus {
            running,
            address,
            error,
        });
    }

    async fn remote_poller(self: Arc<Self>) {
        loop {
            let remotes = self.store.remotes().unwrap_or_default();
            for remote in remotes.into_iter().filter(|r| r.enabled) {
                let result =
                    tokio::time::timeout(REMOTE_POLL_TIMEOUT, remote::poll(&self, &remote))
                        .await
                        .unwrap_or_else(|_| {
                            Err(anyhow::anyhow!(
                                "timed out after {}s (will continue on the next poll)",
                                REMOTE_POLL_TIMEOUT.as_secs()
                            ))
                        });
                let error = result.as_ref().err().map(|e| format!("{e:#}"));
                if let Some(error) = &error {
                    tracing::warn!(remote = %remote.name, %error, "remote mailbox poll failed");
                }
                let changed = error != remote.last_error;
                let _ = self.store.record_remote_poll(&remote.id, error.as_deref());
                if changed {
                    self.emit(Event::RemotesChanged);
                }
            }
            let interval = self.settings().remote_poll_secs.max(5);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
                _ = self.remote_wakeup.notified() => {}
            }
        }
    }

    async fn retention_task(self: Arc<Self>) {
        loop {
            let days = self.settings().retention_days;
            if days > 0 {
                let cutoff = chrono::Utc::now().timestamp() - i64::from(days) * 86_400;
                match self.store.delete_older_than(cutoff) {
                    Ok(ids) if !ids.is_empty() => {
                        self.emit(Event::EmailsDeleted { ids });
                        self.emit(Event::MailboxesChanged);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(error = %format!("{error:#}"), "retention cleanup failed")
                    }
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(3600)) => {}
                _ = self.retention_wakeup.notified() => {}
            }
        }
    }

    pub fn status(&self) -> Status {
        let smtp = self.smtp_state.read().unwrap().clone();
        Status {
            version: VERSION.to_string(),
            smtp_address: if smtp.address.is_empty() {
                self.settings().smtp_address()
            } else {
                smtp.address
            },
            smtp_running: smtp.running,
            smtp_error: smtp.error,
            web_url: self.web_url(),
            data_dir: self.data_dir.display().to_string(),
        }
    }

    pub fn set_web_url(&self, url: Option<String>) {
        *self.web_url.write().unwrap() = url;
    }

    pub fn web_url(&self) -> Option<String> {
        self.web_url.read().unwrap().clone()
    }

    // ---- settings ----------------------------------------------------------

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    /// Persists new settings and restarts services affected by the change.
    /// What to write to `settings.json`: `new`, except that fields overridden at start-up
    /// (CLI flags) keep their saved value unless the user changed them.
    fn persisted(&self, current: &Settings, new: &Settings) -> anyhow::Result<Settings> {
        if self.overrides.is_empty() {
            return Ok(new.clone());
        }
        let current = serde_json::to_value(current)?;
        let mut out = serde_json::to_value(new)?;
        for (key, effective, saved) in &self.overrides {
            if out.get(key) == Some(effective) && current.get(key) == Some(effective) {
                out[key] = saved.clone();
            }
        }
        Ok(serde_json::from_value(out)?)
    }

    pub fn update_settings(&self, new: Settings) -> anyhow::Result<()> {
        let old = {
            // Held while saving so concurrent updates can't persist one and apply the other.
            let mut current = self.settings.write().unwrap();
            self.persisted(&current, &new)?.save(&self.data_dir)?;
            std::mem::replace(&mut *current, new.clone())
        };
        if old.smtp_address() != new.smtp_address() || old.smtp_starttls != new.smtp_starttls {
            self.smtp_restart.notify_one();
        }
        if old.remote_poll_secs != new.remote_poll_secs {
            self.remote_wakeup.notify_one();
        }
        if old.retention_days != new.retention_days {
            self.retention_wakeup.notify_one();
        }
        self.emit(Event::SettingsChanged);
        Ok(())
    }

    // ---- ingest -------------------------------------------------------------

    /// Stores an email delivered over SMTP. Returns its id.
    pub fn ingest_local(
        self: &Arc<Self>,
        raw: Vec<u8>,
        mailbox: &str,
        from: String,
        to: Vec<String>,
    ) -> anyhow::Result<String> {
        self.ingest(raw, mailbox, MailboxKind::Local, from, to, None)
            .map(|id| id.expect("local emails are always inserted"))
    }

    /// Stores an email pulled from a remote mailbox. Returns `None` if it was already imported.
    pub fn ingest_remote(
        self: &Arc<Self>,
        raw: Vec<u8>,
        mailbox: &str,
        from: String,
        to: Vec<String>,
        external_id: String,
    ) -> anyhow::Result<Option<String>> {
        self.ingest(
            raw,
            mailbox,
            MailboxKind::Remote,
            from,
            to,
            Some(external_id),
        )
    }

    fn ingest(
        self: &Arc<Self>,
        raw: Vec<u8>,
        mailbox: &str,
        kind: MailboxKind,
        envelope_from: String,
        envelope_to: Vec<String>,
        external_id: Option<String>,
    ) -> anyhow::Result<Option<String>> {
        let overview = parse::overview(&raw);
        let id = uuid::Uuid::new_v4().simple().to_string();
        let new_mailbox = !self.store.has_mailbox(mailbox)?;
        let inserted = self.store.insert_email(&NewEmail {
            id: id.clone(),
            mailbox: mailbox.to_string(),
            mailbox_kind: kind,
            subject: overview.subject,
            from: if overview.from.is_empty() {
                envelope_from.clone()
            } else {
                overview.from
            },
            to: if overview.to.is_empty() {
                envelope_to.clone()
            } else {
                overview.to
            },
            preview: overview.preview,
            received_at: chrono::Utc::now().timestamp(),
            envelope_from,
            envelope_to,
            has_attachments: overview.has_attachments,
            raw,
            external_id,
        })?;
        if !inserted {
            return Ok(None);
        }
        if new_mailbox {
            self.emit(Event::MailboxesChanged);
        }
        if let Some(summary) = self.store.summary(&id)? {
            self.emit(Event::EmailReceived { email: summary });
        }
        self.schedule_analysis(&id);
        Ok(Some(id))
    }

    /// Runs the link checker and spam report in the background according to settings.
    fn schedule_analysis(self: &Arc<Self>, id: &str) {
        let settings = self.settings();
        if settings.link_checker {
            self.spawn_link_check(id);
        }
        if settings.spam_check {
            self.spawn_spam_check(id);
        }
    }

    fn runtime() -> Option<tokio::runtime::Handle> {
        tokio::runtime::Handle::try_current().ok()
    }

    /// Re-extracts links and checks them in the background.
    pub fn spawn_link_check(self: &Arc<Self>, id: &str) {
        let Some(runtime) = Self::runtime() else {
            return;
        };
        let Ok(Some(raw)) = self.store.raw(id) else {
            return;
        };
        let (html, text) = match parse::parse(&raw) {
            Some(m) => (
                m.body_html(0).map(|h| h.into_owned()),
                m.body_text(0).map(|t| t.into_owned()),
            ),
            None => (None, None),
        };
        let pending = links::pending(&links::extract(html.as_deref(), text.as_deref()));
        let _ = self.store.set_links(id, &pending);
        self.emit(Event::EmailUpdated { id: id.to_string() });

        let timeout = Duration::from_secs(self.settings().link_checker_timeout_secs.max(1));
        let scratchpad = self.clone();
        let id = id.to_string();
        runtime.spawn(async move {
            let checked = links::check_all(&scratchpad.http, pending, timeout).await;
            if scratchpad.store.set_links(&id, &checked).is_ok() {
                scratchpad.emit(Event::EmailUpdated { id });
            }
        });
    }

    /// Runs the spam report in the background.
    pub fn spawn_spam_check(self: &Arc<Self>, id: &str) {
        let Some(runtime) = Self::runtime() else {
            return;
        };
        let Ok(Some(raw)) = self.store.raw(id) else {
            return;
        };
        let settings = self.settings();
        let scratchpad = self.clone();
        let id = id.to_string();
        runtime.spawn(async move {
            let report = spam::analyze(&raw, &settings.spamd_host, settings.spamd_port).await;
            if scratchpad.store.set_spam(&id, &report).is_ok() {
                scratchpad.emit(Event::EmailUpdated { id });
            }
        });
    }

    // ---- queries -------------------------------------------------------------

    pub fn mailboxes(&self) -> anyhow::Result<Vec<Mailbox>> {
        self.store.mailboxes()
    }

    pub fn emails(
        &self,
        mailbox: Option<&str>,
        search: Option<&str>,
    ) -> anyhow::Result<Vec<EmailSummary>> {
        self.store.emails(mailbox, search)
    }

    /// One page of emails (newest first) and the total matching `mailbox`/`search`.
    pub fn emails_page(
        &self,
        mailbox: Option<&str>,
        search: Option<&str>,
        limit: Option<usize>,
        offset: usize,
    ) -> anyhow::Result<(Vec<EmailSummary>, usize)> {
        self.store.emails_page(mailbox, search, limit, offset)
    }

    pub fn emails_since(&self, mailbox: &str, since: i64) -> anyhow::Result<Vec<EmailSummary>> {
        self.store.emails_since(mailbox, since)
    }

    pub fn summary(&self, id: &str) -> anyhow::Result<Option<EmailSummary>> {
        self.store.summary(id)
    }

    pub fn email(&self, id: &str) -> anyhow::Result<Option<EmailDetail>> {
        let Some(summary) = self.store.summary(id)? else {
            return Ok(None);
        };
        let Some(raw) = self.store.raw(id)? else {
            return Ok(None);
        };
        let (envelope_from, envelope_to) = self.store.envelope(id)?.unwrap_or_default();
        let mut detail = parse::detail(&raw, summary, envelope_from, envelope_to);
        let (links, spam) = self.store.analysis(id)?;
        detail.links = links;
        detail.spam = spam;
        Ok(Some(detail))
    }

    pub fn raw(&self, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
        self.store.raw(id)
    }

    pub fn attachment(
        &self,
        id: &str,
        index: usize,
    ) -> anyhow::Result<Option<(Attachment, Vec<u8>)>> {
        Ok(self
            .store
            .raw(id)?
            .and_then(|raw| parse::attachment(&raw, index)))
    }

    // ---- mutations -------------------------------------------------------------

    pub fn mark_read(&self, ids: &[String], read: bool) -> anyhow::Result<()> {
        if self.store.set_read(ids, read)? > 0 {
            for id in ids {
                self.emit(Event::EmailUpdated { id: id.clone() });
            }
            self.emit(Event::MailboxesChanged);
        }
        Ok(())
    }

    pub fn delete_emails(&self, ids: &[String]) -> anyhow::Result<()> {
        let deleted = self.store.delete_emails(ids)?;
        if !deleted.is_empty() {
            self.emit(Event::EmailsDeleted { ids: deleted });
            self.emit(Event::MailboxesChanged);
        }
        Ok(())
    }

    pub fn clear_mailbox(&self, name: &str) -> anyhow::Result<()> {
        let ids = self.store.clear_mailbox(name)?;
        self.emit(Event::EmailsDeleted { ids });
        self.emit(Event::MailboxesChanged);
        Ok(())
    }

    pub fn delete_mailbox(&self, name: &str) -> anyhow::Result<()> {
        let ids = self.store.delete_mailbox(name)?;
        self.emit(Event::EmailsDeleted { ids });
        self.emit(Event::MailboxesChanged);
        Ok(())
    }

    pub fn recheck(self: &Arc<Self>, id: &str) {
        self.spawn_link_check(id);
        self.spawn_spam_check(id);
    }

    pub async fn forward(&self, id: &str, recipients: &[String]) -> anyhow::Result<()> {
        let raw = self.store.raw(id)?.context("email not found")?;
        let (from, _) = self.store.envelope(id)?.unwrap_or_default();
        let from = if from.is_empty() {
            parse::parse(&raw)
                .and_then(|m| {
                    m.from()
                        .and_then(|f| f.first())
                        .and_then(|a| a.address())
                        .map(str::to_string)
                })
                .unwrap_or_default()
        } else {
            from
        };
        forward::forward(&self.settings().forward, &raw, &from, recipients).await
    }

    pub async fn send_test_email(&self, mailbox: &str) -> anyhow::Result<()> {
        let settings = self.settings();
        forward::send_test_email(&settings.smtp_host, settings.smtp_port, mailbox).await
    }

    // ---- shares ------------------------------------------------------------------

    pub fn create_share(&self, title: &str, email_ids: Vec<String>) -> anyhow::Result<Share> {
        if email_ids.is_empty() {
            anyhow::bail!("select at least one email to share");
        }
        self.insert_share(title, email_ids, None)
    }

    /// Shares a whole mailbox, including emails that arrive after the link was created.
    pub fn create_mailbox_share(&self, title: &str, mailbox: &str) -> anyhow::Result<Share> {
        if !self.store.has_mailbox(mailbox)? {
            anyhow::bail!("mailbox \"{mailbox}\" does not exist");
        }
        self.insert_share(title, vec![], Some(mailbox.to_string()))
    }

    fn insert_share(
        &self,
        title: &str,
        email_ids: Vec<String>,
        mailbox: Option<String>,
    ) -> anyhow::Result<Share> {
        let share = Share {
            token: Alphanumeric.sample_string(&mut rand::rng(), 32),
            title: title.to_string(),
            email_ids,
            mailbox,
            created_at: chrono::Utc::now().timestamp(),
        };
        self.store.insert_share(&share)?;
        self.emit(Event::SharesChanged);
        Ok(share)
    }

    /// The emails currently visible through a share.
    pub fn share_emails(&self, share: &Share) -> anyhow::Result<Vec<EmailSummary>> {
        match &share.mailbox {
            Some(mailbox) => self.store.emails(Some(mailbox), None),
            None => Ok(share
                .email_ids
                .iter()
                .filter_map(|id| self.store.summary(id).ok().flatten())
                .collect()),
        }
    }

    /// Whether `id` may be viewed through `share`.
    pub fn share_contains(&self, share: &Share, id: &str) -> anyhow::Result<bool> {
        Ok(match &share.mailbox {
            Some(mailbox) => self
                .store
                .summary(id)?
                .is_some_and(|email| &email.mailbox == mailbox),
            None => share.email_ids.iter().any(|shared| shared == id),
        })
    }

    pub fn shares(&self) -> anyhow::Result<Vec<Share>> {
        self.store.shares()
    }

    pub fn share(&self, token: &str) -> anyhow::Result<Option<Share>> {
        self.store.share(token)
    }

    pub fn delete_share(&self, token: &str) -> anyhow::Result<()> {
        self.store.delete_share(token)?;
        self.emit(Event::SharesChanged);
        Ok(())
    }

    /// The public URL for a share, based on `public_url` or the running web server.
    pub fn share_url(&self, token: &str) -> Option<String> {
        let public = self.settings().public_url;
        let base = if public.trim().is_empty() {
            self.web_url()?
        } else {
            public
        };
        Some(format!("{}/s/{token}", base.trim_end_matches('/')))
    }

    // ---- remote mailboxes -----------------------------------------------------------

    pub fn remotes(&self) -> anyhow::Result<Vec<RemoteMailbox>> {
        self.store.remotes()
    }

    pub fn save_remote(&self, mut remote: RemoteMailbox) -> anyhow::Result<RemoteMailbox> {
        if remote.name.trim().is_empty() {
            anyhow::bail!("remote mailbox needs a name");
        }
        remote.name = remote.name.trim().to_string();
        if remote.id.is_empty() {
            remote.id = uuid::Uuid::new_v4().simple().to_string();
        }
        if self
            .store
            .remotes()?
            .iter()
            .any(|r| r.id != remote.id && r.name == remote.name)
        {
            anyhow::bail!("a remote mailbox named \"{}\" already exists", remote.name);
        }
        self.store.upsert_remote(&remote)?;
        self.store
            .ensure_mailbox(&remote.name, MailboxKind::Remote)?;
        self.emit(Event::RemotesChanged);
        self.emit(Event::MailboxesChanged);
        self.remote_wakeup.notify_one();
        Ok(remote)
    }

    pub fn delete_remote(&self, id: &str, delete_emails: bool) -> anyhow::Result<()> {
        if let Some(name) = self.store.delete_remote(id)? {
            if delete_emails {
                self.delete_mailbox(&name)?;
            }
        }
        self.emit(Event::RemotesChanged);
        Ok(())
    }

    pub fn poll_remotes_now(&self) {
        self.remote_wakeup.notify_one();
    }
}

/// Creates `dir` (and its parents). On Unix a newly created data directory is
/// accessible only by the current user: it holds emails and credentials.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Atomically replaces `path` with `bytes` (write to a temporary file, fsync,
/// rename). On Unix the file is readable only by the current user. On Windows,
/// files in the per-user data directory inherit its ACL.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    // A leftover from a crash may have other permissions; start from scratch.
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("writing {}", tmp.display()))?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::Error::from(error).context(format!("writing {}", path.display())));
    }
    Ok(())
}

/// Top-level settings fields that differ between the saved and the effective settings.
fn setting_overrides(
    saved: &Settings,
    effective: &Settings,
) -> Vec<(String, serde_json::Value, serde_json::Value)> {
    let (Ok(serde_json::Value::Object(saved)), Ok(serde_json::Value::Object(effective))) =
        (serde_json::to_value(saved), serde_json::to_value(effective))
    else {
        return Vec::new();
    };
    effective
        .into_iter()
        .filter_map(|(key, value)| {
            let old = saved.get(&key)?;
            (old != &value).then(|| (key, value, old.clone()))
        })
        .collect()
}
