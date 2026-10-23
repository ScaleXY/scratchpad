use serde::Serialize;

use crate::model::EmailSummary;

/// Change notifications broadcast to every connected UI (GUI, web clients).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Event {
    EmailReceived {
        email: EmailSummary,
    },
    EmailUpdated {
        id: String,
    },
    EmailsDeleted {
        ids: Vec<String>,
    },
    MailboxesChanged,
    SharesChanged,
    RemotesChanged,
    /// S3 configs, the credential vault or cloud shares changed.
    CloudChanged,
    SettingsChanged,
    SmtpStatus {
        running: bool,
        address: String,
        error: Option<String>,
    },
}
