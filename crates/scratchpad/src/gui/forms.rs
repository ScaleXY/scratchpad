//! Editable form state for the settings page, remote mailboxes, S3 configs and the
//! credential vault.

use scratchpad_core::{
    CredentialStorage, ForwardEncryption, RemoteMailbox, RemoteSource, S3Config, S3Mode, Settings,
    ThemePreference,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    SmtpHost,
    SmtpPort,
    MaxSize,
    LinkTimeout,
    SpamdHost,
    SpamdPort,
    FwdHost,
    FwdPort,
    FwdUser,
    FwdPass,
    FwdFrom,
    WebHost,
    WebPort,
    WebUser,
    WebPass,
    PublicUrl,
    PollSecs,
    Retention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    StartTls,
    RequireAuth,
    LinkChecker,
    SpamCheck,
    Notifications,
}

#[derive(Debug, Clone)]
pub enum SettingsMsg {
    Text(Field, String),
    Toggle(Toggle, bool),
    Encryption(ForwardEncryption),
    Theme(ThemePreference),
}

/// Settings being edited. Numbers are kept as text until saved.
pub struct SettingsForm {
    pub draft: Settings,
    pub texts: Vec<(Field, String)>,
}

impl SettingsForm {
    pub fn new(settings: &Settings) -> Self {
        let s = settings;
        let texts = vec![
            (Field::SmtpHost, s.smtp_host.clone()),
            (Field::SmtpPort, s.smtp_port.to_string()),
            (Field::MaxSize, s.max_message_size.to_string()),
            (Field::LinkTimeout, s.link_checker_timeout_secs.to_string()),
            (Field::SpamdHost, s.spamd_host.clone()),
            (Field::SpamdPort, s.spamd_port.to_string()),
            (Field::FwdHost, s.forward.host.clone()),
            (Field::FwdPort, s.forward.port.to_string()),
            (Field::FwdUser, s.forward.username.clone()),
            (Field::FwdPass, s.forward.password.clone()),
            (Field::FwdFrom, s.forward.from_override.clone()),
            (Field::WebHost, s.web_host.clone()),
            (Field::WebPort, s.web_port.to_string()),
            (Field::WebUser, s.web_username.clone()),
            (Field::WebPass, s.web_password.clone()),
            (Field::PublicUrl, s.public_url.clone()),
            (Field::PollSecs, s.remote_poll_secs.to_string()),
            (Field::Retention, s.retention_days.to_string()),
        ];
        Self {
            draft: settings.clone(),
            texts,
        }
    }

    pub fn text(&self, field: Field) -> &str {
        self.texts
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, v)| v.as_str())
            .unwrap_or_default()
    }

    pub fn toggle(&self, toggle: Toggle) -> bool {
        let d = &self.draft;
        match toggle {
            Toggle::StartTls => d.smtp_starttls,
            Toggle::RequireAuth => d.smtp_require_auth,
            Toggle::LinkChecker => d.link_checker,
            Toggle::SpamCheck => d.spam_check,
            Toggle::Notifications => d.notifications,
        }
    }

    pub fn update(&mut self, msg: SettingsMsg) {
        match msg {
            SettingsMsg::Text(field, value) => {
                if let Some(entry) = self.texts.iter_mut().find(|(f, _)| *f == field) {
                    entry.1 = value;
                }
            }
            SettingsMsg::Toggle(toggle, value) => {
                let d = &mut self.draft;
                match toggle {
                    Toggle::StartTls => d.smtp_starttls = value,
                    Toggle::RequireAuth => d.smtp_require_auth = value,
                    Toggle::LinkChecker => d.link_checker = value,
                    Toggle::SpamCheck => d.spam_check = value,
                    Toggle::Notifications => d.notifications = value,
                }
            }
            SettingsMsg::Encryption(encryption) => {
                self.draft.forward.encryption = encryption;
                self.draft.forward.port = match encryption {
                    ForwardEncryption::Tls => 465,
                    ForwardEncryption::StartTls => 587,
                    ForwardEncryption::None => 25,
                };
                self.update(SettingsMsg::Text(
                    Field::FwdPort,
                    self.draft.forward.port.to_string(),
                ));
            }
            SettingsMsg::Theme(theme) => self.draft.theme = theme,
        }
    }

    pub fn build(&self) -> anyhow::Result<Settings> {
        let port = |field: Field, label: &str| port(self.text(field), label);
        let mut s = self.draft.clone();
        s.smtp_host = self.text(Field::SmtpHost).trim().to_string();
        s.smtp_port = port(Field::SmtpPort, "SMTP port")?;
        s.max_message_size = number(self.text(Field::MaxSize), "Max message size")?;
        s.link_checker_timeout_secs = number(self.text(Field::LinkTimeout), "Link timeout")?;
        s.spamd_host = self.text(Field::SpamdHost).trim().to_string();
        s.spamd_port = port(Field::SpamdPort, "spamd port")?;
        s.forward.host = self.text(Field::FwdHost).trim().to_string();
        s.forward.port = port(Field::FwdPort, "Forwarding port")?;
        s.forward.username = self.text(Field::FwdUser).to_string();
        s.forward.password = self.text(Field::FwdPass).to_string();
        s.forward.from_override = self.text(Field::FwdFrom).trim().to_string();
        s.web_host = self.text(Field::WebHost).trim().to_string();
        s.web_port = port(Field::WebPort, "Web port")?;
        s.web_username = self.text(Field::WebUser).to_string();
        s.web_password = self.text(Field::WebPass).to_string();
        s.public_url = self
            .text(Field::PublicUrl)
            .trim()
            .trim_end_matches('/')
            .to_string();
        s.remote_poll_secs = number(self.text(Field::PollSecs), "Remote poll interval")?;
        s.retention_days = number(self.text(Field::Retention), "Delete emails after")?;
        if s.smtp_host.is_empty() {
            anyhow::bail!("SMTP host cannot be empty");
        }
        if s.web_host.is_empty() {
            anyhow::bail!("Web interface host cannot be empty");
        }
        if s.max_message_size == 0 {
            anyhow::bail!("Max message size must be greater than 0");
        }
        if s.link_checker_timeout_secs == 0 {
            anyhow::bail!("Link timeout must be at least 1 second");
        }
        if s.remote_poll_secs < 5 {
            anyhow::bail!("Remote poll interval must be at least 5 seconds");
        }
        if !s.public_url.is_empty() && !is_http_url(&s.public_url) {
            anyhow::bail!("Public URL must start with http:// or https://");
        }
        Ok(s)
    }
}

/// A non-negative whole number.
fn number<T: std::str::FromStr>(value: &str, label: &str) -> anyhow::Result<T> {
    value
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{label} must be a whole number"))
}

/// A TCP port, 1–65535.
fn port(value: &str, label: &str) -> anyhow::Result<u16> {
    match value.trim().parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => anyhow::bail!("{label} must be a number from 1 to 65535"),
    }
}

/// `http(s)://` followed by a host.
fn is_http_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    ["http://", "https://"].iter().any(|scheme| {
        lower
            .strip_prefix(scheme)
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteKind {
    Imap,
    Scratchpad,
}

impl std::fmt::Display for RemoteKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RemoteKind::Imap => "IMAP account",
            RemoteKind::Scratchpad => "Scratchpad server",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteField {
    Name,
    Host,
    Port,
    Username,
    Password,
    Folder,
    BaseUrl,
    Mailbox,
}

#[derive(Debug, Clone)]
pub enum RemoteMsg {
    Text(RemoteField, String),
    Kind(RemoteKind),
    Tls(bool),
    DeleteAfterFetch(bool),
    Enabled(bool),
}

#[derive(Debug, Clone)]
pub struct RemoteForm {
    pub id: String,
    pub name: String,
    pub kind: RemoteKind,
    pub enabled: bool,
    pub host: String,
    pub port: String,
    pub tls: bool,
    pub username: String,
    pub password: String,
    pub folder: String,
    pub delete_after_fetch: bool,
    pub base_url: String,
    pub mailbox: String,
}

impl Default for RemoteForm {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: RemoteKind::Imap,
            enabled: true,
            host: String::new(),
            port: "993".into(),
            tls: true,
            username: String::new(),
            password: String::new(),
            folder: "INBOX".into(),
            delete_after_fetch: false,
            base_url: String::new(),
            mailbox: "Default".into(),
        }
    }
}

impl RemoteForm {
    pub fn from_remote(remote: &RemoteMailbox) -> Self {
        let mut form = Self {
            id: remote.id.clone(),
            name: remote.name.clone(),
            enabled: remote.enabled,
            ..Self::default()
        };
        match &remote.source {
            RemoteSource::Imap {
                host,
                port,
                tls,
                username,
                password,
                folder,
                delete_after_fetch,
            } => {
                form.kind = RemoteKind::Imap;
                form.host = host.clone();
                form.port = port.to_string();
                form.tls = *tls;
                form.username = username.clone();
                form.password = password.clone();
                form.folder = folder.clone();
                form.delete_after_fetch = *delete_after_fetch;
            }
            RemoteSource::Scratchpad {
                base_url,
                username,
                password,
                mailbox,
            } => {
                form.kind = RemoteKind::Scratchpad;
                form.base_url = base_url.clone();
                form.username = username.clone().unwrap_or_default();
                form.password = password.clone().unwrap_or_default();
                form.mailbox = mailbox.clone();
            }
        }
        form
    }

    pub fn update(&mut self, msg: RemoteMsg) {
        match msg {
            RemoteMsg::Text(field, value) => {
                let target = match field {
                    RemoteField::Name => &mut self.name,
                    RemoteField::Host => &mut self.host,
                    RemoteField::Port => &mut self.port,
                    RemoteField::Username => &mut self.username,
                    RemoteField::Password => &mut self.password,
                    RemoteField::Folder => &mut self.folder,
                    RemoteField::BaseUrl => &mut self.base_url,
                    RemoteField::Mailbox => &mut self.mailbox,
                };
                *target = value;
            }
            RemoteMsg::Kind(kind) => self.kind = kind,
            RemoteMsg::Tls(tls) => {
                self.tls = tls;
                if self.port == "993" || self.port == "143" {
                    self.port = if tls { "993".into() } else { "143".into() };
                }
            }
            RemoteMsg::DeleteAfterFetch(value) => self.delete_after_fetch = value,
            RemoteMsg::Enabled(value) => self.enabled = value,
        }
    }

    pub fn build(&self) -> anyhow::Result<RemoteMailbox> {
        let name = self.name.trim();
        if name.is_empty() {
            anyhow::bail!("Give the remote mailbox a name");
        }
        let source = match self.kind {
            RemoteKind::Imap => {
                let host = self.host.trim();
                if host.is_empty() {
                    anyhow::bail!("Enter the IMAP host");
                }
                RemoteSource::Imap {
                    host: host.to_string(),
                    port: port(&self.port, "Port")?,
                    tls: self.tls,
                    username: self.username.trim().to_string(),
                    password: self.password.clone(),
                    folder: if self.folder.trim().is_empty() {
                        "INBOX".into()
                    } else {
                        self.folder.trim().to_string()
                    },
                    delete_after_fetch: self.delete_after_fetch,
                }
            }
            RemoteKind::Scratchpad => {
                let base_url = self.base_url.trim().trim_end_matches('/');
                if !is_http_url(base_url) {
                    anyhow::bail!("Server URL must start with http:// or https://");
                }
                RemoteSource::Scratchpad {
                    base_url: base_url.to_string(),
                    username: Some(self.username.trim().to_string()).filter(|s| !s.is_empty()),
                    password: Some(self.password.clone()).filter(|s| !s.is_empty()),
                    mailbox: match self.mailbox.trim() {
                        "" => "Default".into(),
                        mailbox => mailbox.to_string(),
                    },
                }
            }
        };
        Ok(RemoteMailbox {
            id: self.id.clone(),
            name: name.to_string(),
            source,
            enabled: self.enabled,
            last_poll_at: None,
            last_error: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3Field {
    Name,
    Endpoint,
    Region,
    Bucket,
    AccessKey,
    SecretKey,
    CdnDomain,
}

#[derive(Debug, Clone)]
pub enum S3Msg {
    Text(S3Field, String),
    Mode(S3Mode),
    PathStyle(bool),
    Storage(CredentialStorage),
}

/// An S3 config being added or edited. When editing, an empty secret keeps the stored one.
#[derive(Debug, Clone)]
pub struct S3Form {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub cdn_domain: String,
    pub mode: S3Mode,
    pub path_style: bool,
    pub storage: CredentialStorage,
}

impl Default for S3Form {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            endpoint: String::new(),
            region: "auto".into(),
            bucket: String::new(),
            access_key: String::new(),
            secret_key: String::new(),
            cdn_domain: String::new(),
            mode: S3Mode::Public,
            path_style: true,
            storage: CredentialStorage::Local,
        }
    }
}

impl S3Form {
    pub fn from_config(config: &S3Config) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            endpoint: config.endpoint.clone(),
            region: config.region.clone(),
            bucket: config.bucket.clone(),
            access_key: config.access_key.clone(),
            secret_key: String::new(),
            cdn_domain: config.cdn_domain.clone(),
            mode: config.mode,
            path_style: config.path_style,
            storage: config.storage,
        }
    }

    pub fn is_new(&self) -> bool {
        self.id.is_empty()
    }

    pub fn update(&mut self, msg: S3Msg) {
        match msg {
            S3Msg::Text(field, value) => {
                let target = match field {
                    S3Field::Name => &mut self.name,
                    S3Field::Endpoint => &mut self.endpoint,
                    S3Field::Region => &mut self.region,
                    S3Field::Bucket => &mut self.bucket,
                    S3Field::AccessKey => &mut self.access_key,
                    S3Field::SecretKey => &mut self.secret_key,
                    S3Field::CdnDomain => &mut self.cdn_domain,
                };
                *target = value;
            }
            S3Msg::Mode(mode) => self.mode = mode,
            S3Msg::PathStyle(value) => self.path_style = value,
            S3Msg::Storage(storage) => self.storage = storage,
        }
    }

    /// Quick checks before saving or testing, so mistakes show up without a round trip.
    /// The core validates again when saving.
    pub fn validate(&self, for_test: bool) -> Result<(), String> {
        if !for_test && self.name.trim().is_empty() {
            return Err("Give the bucket a name".into());
        }
        let endpoint = self.endpoint.trim();
        if endpoint.is_empty() {
            return Err("Enter the S3 endpoint".into());
        }
        if endpoint.contains("://") && !is_http_url(endpoint) {
            return Err("The S3 endpoint must be an http(s) URL".into());
        }
        if self.bucket.trim().is_empty() {
            return Err("Enter the bucket name".into());
        }
        if self.access_key.trim().is_empty() {
            return Err("Enter the access key".into());
        }
        if self.is_new() && self.secret_key.trim().is_empty() {
            return Err("Enter the secret key".into());
        }
        Ok(())
    }

    /// The config to save or test.
    pub fn build(&self) -> S3Config {
        S3Config {
            id: self.id.clone(),
            name: self.name.trim().to_string(),
            endpoint: self.endpoint.trim().to_string(),
            region: match self.region.trim() {
                "" => "auto".into(),
                region => region.to_string(),
            },
            bucket: self.bucket.trim().to_string(),
            access_key: self.access_key.trim().to_string(),
            secret_key: self.secret_key.trim().to_string(),
            cdn_domain: self.cdn_domain.trim().to_string(),
            mode: self.mode,
            path_style: self.path_style,
            storage: self.storage,
        }
    }
}

/// What the credential vault form is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultAction {
    Create,
    Unlock,
    ChangePassword,
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultField {
    Password,
    NewPassword,
    Confirm,
}

/// Password entry for the credential vault.
#[derive(Debug, Clone)]
pub struct VaultForm {
    pub action: VaultAction,
    pub password: String,
    pub new_password: String,
    pub confirm: String,
}

pub const MIN_MASTER_PASSWORD: usize = 8;

impl VaultForm {
    pub fn new(action: VaultAction) -> Self {
        Self {
            action,
            password: String::new(),
            new_password: String::new(),
            confirm: String::new(),
        }
    }

    pub fn set(&mut self, field: VaultField, value: String) {
        match field {
            VaultField::Password => self.password = value,
            VaultField::NewPassword => self.new_password = value,
            VaultField::Confirm => self.confirm = value,
        }
    }

    /// Checks the input; returns the error to show, if any.
    pub fn validate(&self) -> Result<(), String> {
        let new = match self.action {
            VaultAction::Create => &self.password,
            VaultAction::ChangePassword => &self.new_password,
            VaultAction::Unlock | VaultAction::Restore => {
                if self.password.is_empty() {
                    return Err("Enter the master password".into());
                }
                return Ok(());
            }
        };
        if self.action == VaultAction::ChangePassword && self.password.is_empty() {
            return Err("Enter the current master password".into());
        }
        if new.chars().count() < MIN_MASTER_PASSWORD {
            return Err(format!(
                "The master password needs at least {MIN_MASTER_PASSWORD} characters"
            ));
        }
        if *new != self.confirm {
            return Err("The passwords don't match".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> SettingsForm {
        SettingsForm::new(&Settings::default())
    }

    fn set(form: &mut SettingsForm, field: Field, value: &str) {
        form.update(SettingsMsg::Text(field, value.into()));
    }

    fn error(form: &SettingsForm) -> String {
        form.build().unwrap_err().to_string()
    }

    #[test]
    fn settings_round_trip() {
        let settings = Settings::default();
        let built = SettingsForm::new(&settings).build().unwrap();
        assert_eq!(built.smtp_port, settings.smtp_port);
        assert_eq!(built.web_port, settings.web_port);
        assert_eq!(built.retention_days, settings.retention_days);
    }

    #[test]
    fn settings_parse_and_trim() {
        let mut f = form();
        set(&mut f, Field::SmtpHost, "  0.0.0.0 ");
        set(&mut f, Field::SmtpPort, " 2526 ");
        set(&mut f, Field::Retention, "7");
        set(
            &mut f,
            Field::PublicUrl,
            " https://scratchpad.example.com/ ",
        );
        f.update(SettingsMsg::Toggle(Toggle::LinkChecker, false));
        let s = f.build().unwrap();
        assert_eq!(s.smtp_host, "0.0.0.0");
        assert_eq!(s.smtp_port, 2526);
        assert_eq!(s.retention_days, 7);
        assert_eq!(s.public_url, "https://scratchpad.example.com");
        assert!(!s.link_checker);
    }

    #[test]
    fn settings_validation_messages() {
        let mut f = form();
        set(&mut f, Field::SmtpPort, "abc");
        assert!(error(&f).contains("SMTP port must be a number from 1 to 65535"));
        set(&mut f, Field::SmtpPort, "0");
        assert!(error(&f).contains("SMTP port"));
        set(&mut f, Field::SmtpPort, "70000");
        assert!(error(&f).contains("SMTP port"));
        set(&mut f, Field::SmtpPort, "2525");

        set(&mut f, Field::Retention, "-1");
        assert!(error(&f).contains("Delete emails after must be a whole number"));
        set(&mut f, Field::Retention, "0");

        set(&mut f, Field::PollSecs, "1");
        assert!(error(&f).contains("at least 5 seconds"));
        set(&mut f, Field::PollSecs, "30");

        set(&mut f, Field::LinkTimeout, "0");
        assert!(error(&f).contains("Link timeout"));
        set(&mut f, Field::LinkTimeout, "5");

        set(&mut f, Field::SmtpHost, "  ");
        assert!(error(&f).contains("SMTP host"));
        set(&mut f, Field::SmtpHost, "127.0.0.1");

        set(&mut f, Field::PublicUrl, "scratchpad.example.com");
        assert!(error(&f).contains("Public URL"));
        set(&mut f, Field::PublicUrl, "");
        assert!(f.build().is_ok());
    }

    #[test]
    fn forward_encryption_sets_default_port() {
        let mut f = form();
        f.update(SettingsMsg::Encryption(ForwardEncryption::Tls));
        assert_eq!(f.text(Field::FwdPort), "465");
        f.update(SettingsMsg::Encryption(ForwardEncryption::StartTls));
        assert_eq!(f.build().unwrap().forward.port, 587);
    }

    #[test]
    fn remote_imap() {
        let mut f = RemoteForm::default();
        assert!(f.build().unwrap_err().to_string().contains("name"));
        f.update(RemoteMsg::Text(RemoteField::Name, " Staging ".into()));
        assert!(f.build().unwrap_err().to_string().contains("IMAP host"));
        f.update(RemoteMsg::Text(
            RemoteField::Host,
            "imap.example.com".into(),
        ));
        f.update(RemoteMsg::Text(RemoteField::Folder, " ".into()));
        f.update(RemoteMsg::Tls(false));
        assert_eq!(f.port, "143");
        f.update(RemoteMsg::Text(RemoteField::Port, "99999".into()));
        assert!(f.build().unwrap_err().to_string().contains("Port"));
        f.update(RemoteMsg::Text(RemoteField::Port, "1143".into()));
        let remote = f.build().unwrap();
        assert_eq!(remote.name, "Staging");
        match &remote.source {
            RemoteSource::Imap {
                port, folder, tls, ..
            } => {
                assert_eq!((*port, folder.as_str(), *tls), (1143, "INBOX", false));
            }
            other => panic!("unexpected source {other:?}"),
        }
        // Editing keeps the values.
        let again = RemoteForm::from_remote(&remote);
        assert_eq!(again.port, "1143");
        assert_eq!(again.host, "imap.example.com");
    }

    #[test]
    fn remote_scratchpad() {
        let mut f = RemoteForm::default();
        f.update(RemoteMsg::Kind(RemoteKind::Scratchpad));
        f.update(RemoteMsg::Text(RemoteField::Name, "Prod".into()));
        f.update(RemoteMsg::Text(
            RemoteField::BaseUrl,
            "scratchpad.example.com".into(),
        ));
        assert!(f.build().unwrap_err().to_string().contains("Server URL"));
        f.update(RemoteMsg::Text(
            RemoteField::BaseUrl,
            "https://scratchpad.example.com/".into(),
        ));
        f.update(RemoteMsg::Text(RemoteField::Mailbox, "".into()));
        match f.build().unwrap().source {
            RemoteSource::Scratchpad {
                base_url,
                username,
                mailbox,
                ..
            } => {
                assert_eq!(base_url, "https://scratchpad.example.com");
                assert_eq!(username, None);
                assert_eq!(mailbox, "Default");
            }
            other => panic!("unexpected source {other:?}"),
        }
    }

    #[test]
    fn s3_form_validation() {
        let mut f = S3Form::default();
        let text = |f: &mut S3Form, field, value: &str| f.update(S3Msg::Text(field, value.into()));
        assert_eq!(f.validate(false).unwrap_err(), "Give the bucket a name");
        // Testing doesn't need a name.
        assert_eq!(f.validate(true).unwrap_err(), "Enter the S3 endpoint");
        text(&mut f, S3Field::Name, "R2");
        text(&mut f, S3Field::Endpoint, "ftp://example.com");
        assert!(f.validate(false).unwrap_err().contains("http(s)"));
        text(&mut f, S3Field::Endpoint, "s3.eu-west-1.amazonaws.com");
        assert_eq!(f.validate(false).unwrap_err(), "Enter the bucket name");
        text(&mut f, S3Field::Bucket, "mails");
        assert_eq!(f.validate(false).unwrap_err(), "Enter the access key");
        text(&mut f, S3Field::AccessKey, "key");
        assert_eq!(f.validate(false).unwrap_err(), "Enter the secret key");
        text(&mut f, S3Field::SecretKey, "secret");
        assert_eq!(f.validate(false), Ok(()));

        text(&mut f, S3Field::Region, "  ");
        let config = f.build();
        assert_eq!(config.region, "auto");

        // Editing an existing config: an empty secret keeps the stored one.
        let mut edit = S3Form::from_config(&S3Config {
            id: "abc".into(),
            ..config
        });
        assert!(edit.secret_key.is_empty());
        assert_eq!(edit.validate(false), Ok(()));
        edit.update(S3Msg::Mode(S3Mode::Private));
        assert_eq!(edit.build().mode, S3Mode::Private);
    }

    #[test]
    fn vault_form_validation() {
        let mut f = VaultForm::new(VaultAction::Create);
        f.set(VaultField::Password, "short".into());
        assert!(f.validate().unwrap_err().contains("at least 8"));
        f.set(VaultField::Password, "correct horse".into());
        f.set(VaultField::Confirm, "correct horsy".into());
        assert!(f.validate().unwrap_err().contains("don't match"));
        f.set(VaultField::Confirm, "correct horse".into());
        assert!(f.validate().is_ok());

        let mut change = VaultForm::new(VaultAction::ChangePassword);
        change.set(VaultField::NewPassword, "new password".into());
        change.set(VaultField::Confirm, "new password".into());
        assert!(change.validate().unwrap_err().contains("current"));
        change.set(VaultField::Password, "old".into());
        assert!(change.validate().is_ok());

        let unlock = VaultForm::new(VaultAction::Unlock);
        assert!(unlock.validate().is_err());
    }
}
