//! Plain data types shared by the core, the web API and the GUI.

use serde::{Deserialize, Serialize};

/// The mailbox used when a client delivers mail without SMTP authentication.
pub const DEFAULT_MAILBOX: &str = "Default";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mailbox {
    pub name: String,
    pub kind: MailboxKind,
    pub total: u64,
    pub unread: u64,
    pub created_at: i64,
}

/// A lightweight row used by email lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmailSummary {
    pub id: String,
    pub mailbox: String,
    pub subject: String,
    pub from: String,
    pub to: Vec<String>,
    pub preview: String,
    pub received_at: i64,
    pub size: u64,
    pub read: bool,
    pub has_attachments: bool,
    pub spam_score: Option<f64>,
    pub broken_links: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.name {
            Some(name) if !name.is_empty() => write!(f, "{name} <{}>", self.email),
            _ => f.write_str(&self.email),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub index: usize,
    pub filename: String,
    pub content_type: String,
    pub content_id: Option<String>,
    pub inline: bool,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeaderField {
    pub name: String,
    pub value: String,
}

/// Debug information added by the `beyondcode/helo-laravel` helper package
/// through the `X-HELO-View`, `X-HELO-View-File` and `X-HELO-View-Data` headers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewInspector {
    pub view_file: Option<String>,
    pub view_source: Option<String>,
    /// HTML produced by Symfony's `HtmlDumper`.
    pub view_data_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum LinkStatus {
    Pending,
    Ok { status: u16, method: String },
    Broken { status: Option<u16>, error: String },
    Skipped { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkCheck {
    pub url: String,
    pub text: Option<String>,
    pub status: LinkStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpamRule {
    pub name: String,
    pub score: f64,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpamReport {
    pub score: f64,
    pub threshold: f64,
    pub is_spam: bool,
    pub rules: Vec<SpamRule>,
    /// `spamassassin` when produced by spamd, `builtin` for the bundled heuristics.
    pub engine: String,
    pub note: Option<String>,
}

/// Everything the UIs need to render one email.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmailDetail {
    pub summary: EmailSummary,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub envelope_from: String,
    pub envelope_to: Vec<String>,
    pub date: Option<String>,
    pub message_id: Option<String>,
    pub html: Option<String>,
    pub text: Option<String>,
    pub headers: Vec<HeaderField>,
    pub attachments: Vec<Attachment>,
    pub view_inspector: Option<ViewInspector>,
    pub links: Option<Vec<LinkCheck>>,
    pub spam: Option<SpamReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Share {
    pub token: String,
    pub title: String,
    pub email_ids: Vec<String>,
    /// When set, the share follows this mailbox live (including emails that arrive later)
    /// and `email_ids` is unused.
    #[serde(default)]
    pub mailbox: Option<String>,
    pub created_at: i64,
}

/// An email included in a cloud share (kept so the list survives deleting it locally).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedEmail {
    pub id: String,
    pub subject: String,
}

/// An exported email (or several) uploaded to an S3 bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudShare {
    pub id: String,
    pub title: String,
    pub emails: Vec<SharedEmail>,
    pub config_id: String,
    pub config_name: String,
    pub object_key: String,
    pub visibility: crate::s3::Visibility,
    /// The public link, or the latest presigned link for private shares.
    pub url: String,
    /// When the presigned link stops working (private shares).
    pub expires_at: Option<i64>,
    pub size: u64,
    pub created_at: i64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RemoteSource {
    /// Poll an IMAP account (a real address that receives test mail).
    Imap {
        host: String,
        port: u16,
        tls: bool,
        username: String,
        password: String,
        #[serde(default = "default_folder")]
        folder: String,
        #[serde(default)]
        delete_after_fetch: bool,
    },
    /// Pull from another Scratchpad instance running headless (e.g. on a staging server).
    Scratchpad {
        base_url: String,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
        mailbox: String,
    },
}

impl std::fmt::Debug for RemoteSource {
    /// Passwords are redacted so they can't end up in logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::settings::redacted;
        match self {
            RemoteSource::Imap {
                host,
                port,
                tls,
                username,
                password,
                folder,
                delete_after_fetch,
            } => f
                .debug_struct("Imap")
                .field("host", host)
                .field("port", port)
                .field("tls", tls)
                .field("username", username)
                .field("password", &redacted(password))
                .field("folder", folder)
                .field("delete_after_fetch", delete_after_fetch)
                .finish(),
            RemoteSource::Scratchpad {
                base_url,
                username,
                password,
                mailbox,
            } => f
                .debug_struct("Scratchpad")
                .field("base_url", base_url)
                .field("username", username)
                .field("password", &password.as_deref().map(redacted))
                .field("mailbox", mailbox)
                .finish(),
        }
    }
}

fn default_folder() -> String {
    "INBOX".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteMailbox {
    pub id: String,
    pub name: String,
    pub source: RemoteSource,
    pub enabled: bool,
    pub last_poll_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub version: String,
    pub smtp_address: String,
    pub smtp_running: bool,
    pub smtp_error: Option<String>,
    pub web_url: Option<String>,
    pub data_dir: String,
}
