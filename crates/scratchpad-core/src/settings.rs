use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ForwardEncryption {
    #[default]
    None,
    StartTls,
    Tls,
}

/// SMTP server used to forward captured emails to real inboxes.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ForwardSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub encryption: ForwardEncryption,
    /// Envelope sender used when forwarding. Empty keeps the original sender.
    pub from_override: String,
}

impl Default for ForwardSettings {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 587,
            username: String::new(),
            password: String::new(),
            encryption: ForwardEncryption::StartTls,
            from_override: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub smtp_host: String,
    pub smtp_port: u16,
    /// Advertise STARTTLS with a self-signed certificate.
    pub smtp_starttls: bool,
    /// Reject unauthenticated deliveries instead of using the "Default" mailbox.
    pub smtp_require_auth: bool,
    pub max_message_size: u64,

    pub web_host: String,
    pub web_port: u16,
    /// Optional HTTP basic auth for the web interface.
    pub web_username: String,
    pub web_password: String,
    /// Public base URL used for share links (e.g. `https://scratchpad.example.com`).
    pub public_url: String,

    pub link_checker: bool,
    pub link_checker_timeout_secs: u64,
    pub spam_check: bool,
    pub spamd_host: String,
    pub spamd_port: u16,

    pub notifications: bool,
    pub theme: ThemePreference,
    pub remote_poll_secs: u64,
    /// Delete emails older than this many days (0 keeps everything).
    pub retention_days: u32,

    pub forward: ForwardSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            smtp_host: "127.0.0.1".into(),
            smtp_port: 2525,
            smtp_starttls: false,
            smtp_require_auth: false,
            max_message_size: 50 * 1024 * 1024,
            web_host: "127.0.0.1".into(),
            web_port: 8025,
            web_username: String::new(),
            web_password: String::new(),
            public_url: String::new(),
            link_checker: true,
            link_checker_timeout_secs: 10,
            spam_check: true,
            spamd_host: "127.0.0.1".into(),
            spamd_port: 783,
            notifications: true,
            theme: ThemePreference::System,
            remote_poll_secs: 15,
            retention_days: 0,
            forward: ForwardSettings::default(),
        }
    }
}

/// Shown instead of passwords in `Debug` output, so they can't end up in logs.
pub(crate) fn redacted(secret: &str) -> &'static str {
    if secret.is_empty() {
        ""
    } else {
        "<redacted>"
    }
}

impl std::fmt::Debug for ForwardSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &redacted(&self.password))
            .field("encryption", &self.encryption)
            .field("from_override", &self.from_override)
            .finish()
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("smtp_starttls", &self.smtp_starttls)
            .field("smtp_require_auth", &self.smtp_require_auth)
            .field("max_message_size", &self.max_message_size)
            .field("web_host", &self.web_host)
            .field("web_port", &self.web_port)
            .field("web_username", &self.web_username)
            .field("web_password", &redacted(&self.web_password))
            .field("public_url", &self.public_url)
            .field("link_checker", &self.link_checker)
            .field("link_checker_timeout_secs", &self.link_checker_timeout_secs)
            .field("spam_check", &self.spam_check)
            .field("spamd_host", &self.spamd_host)
            .field("spamd_port", &self.spamd_port)
            .field("notifications", &self.notifications)
            .field("theme", &self.theme)
            .field("remote_poll_secs", &self.remote_poll_secs)
            .field("retention_days", &self.retention_days)
            .field("forward", &self.forward)
            .finish()
    }
}

impl Settings {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("settings.json")
    }

    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let path = Self::path(data_dir);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        // Missing fields take their defaults and unknown fields are ignored, so files
        // written by older or newer versions still load.
        Ok(serde_json::from_str(&raw)?)
    }

    /// Like [`Settings::load`], but an unreadable or corrupt file is moved aside to
    /// `settings.json.corrupt` (so saving the defaults later doesn't destroy it) and
    /// the defaults are returned.
    pub fn load_or_recover(data_dir: &Path) -> Self {
        match Self::load(data_dir) {
            Ok(settings) => settings,
            Err(error) => {
                let path = Self::path(data_dir);
                let backup = path.with_extension("json.corrupt");
                match std::fs::rename(&path, &backup) {
                    Ok(()) => tracing::warn!(
                        error = %format!("{error:#}"),
                        backup = %backup.display(),
                        "settings file is invalid, using defaults"
                    ),
                    Err(_) => tracing::warn!(
                        error = %format!("{error:#}"),
                        "settings file is invalid, using defaults"
                    ),
                }
                Self::default()
            }
        }
    }

    /// Atomically writes `settings.json`. It contains passwords, so on Unix it is
    /// readable only by the current user.
    pub fn save(&self, data_dir: &Path) -> anyhow::Result<()> {
        crate::write_private(&Self::path(data_dir), &serde_json::to_vec_pretty(self)?)
    }

    /// The maximum accepted message size in bytes (`0` means the default, 50 MB).
    pub fn message_size_limit(&self) -> u64 {
        match self.max_message_size {
            0 => Self::default().max_message_size,
            size => size,
        }
    }

    pub fn smtp_address(&self) -> String {
        format!("{}:{}", self.smtp_host, self.smtp_port)
    }

    pub fn web_address(&self) -> String {
        format!("{}:{}", self.web_host, self.web_port)
    }

    pub fn web_auth_enabled(&self) -> bool {
        !self.web_username.is_empty() || !self.web_password.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_passwords() {
        let settings = Settings {
            web_password: "web-secret".into(),
            forward: ForwardSettings {
                password: "smtp-secret".into(),
                ..ForwardSettings::default()
            },
            ..Settings::default()
        };
        let debug = format!("{settings:?}");
        assert!(!debug.contains("web-secret") && !debug.contains("smtp-secret"));
        assert!(debug.contains("<redacted>"));
        let remote = crate::model::RemoteSource::Imap {
            host: "h".into(),
            port: 1,
            tls: true,
            username: "u".into(),
            password: "imap-secret".into(),
            folder: "INBOX".into(),
            delete_after_fetch: false,
        };
        assert!(!format!("{remote:?}").contains("imap-secret"));
    }

    #[test]
    fn size_limit_defaults_when_zero() {
        let mut settings = Settings::default();
        assert_eq!(settings.message_size_limit(), 50 * 1024 * 1024);
        settings.max_message_size = 0;
        assert_eq!(settings.message_size_limit(), 50 * 1024 * 1024);
        settings.max_message_size = 10;
        assert_eq!(settings.message_size_limit(), 10);
    }
}
