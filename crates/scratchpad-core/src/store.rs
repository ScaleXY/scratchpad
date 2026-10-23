//! SQLite persistence for mailboxes, emails, shares and remote mailboxes.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::model::{
    CloudShare, EmailSummary, LinkCheck, Mailbox, MailboxKind, RemoteMailbox, RemoteSource, Share,
    SpamReport,
};

pub struct Store {
    conn: Mutex<Connection>,
}

/// A row ready to be inserted, built from [`crate::parse::overview`].
pub struct NewEmail {
    pub id: String,
    pub mailbox: String,
    pub mailbox_kind: MailboxKind,
    pub subject: String,
    pub from: String,
    pub to: Vec<String>,
    pub preview: String,
    pub received_at: i64,
    pub envelope_from: String,
    pub envelope_to: Vec<String>,
    pub has_attachments: bool,
    pub raw: Vec<u8>,
    /// Remote mailboxes use this to avoid importing the same message twice.
    pub external_id: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS mailboxes (
    name TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS emails (
    id TEXT PRIMARY KEY,
    mailbox TEXT NOT NULL REFERENCES mailboxes(name) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    from_addr TEXT NOT NULL,
    to_json TEXT NOT NULL,
    preview TEXT NOT NULL,
    received_at INTEGER NOT NULL,
    envelope_from TEXT NOT NULL,
    envelope_to_json TEXT NOT NULL,
    size INTEGER NOT NULL,
    has_attachments INTEGER NOT NULL,
    read INTEGER NOT NULL DEFAULT 0,
    raw BLOB NOT NULL,
    links_json TEXT,
    spam_json TEXT,
    external_id TEXT
);
CREATE TABLE IF NOT EXISTS shares (
    token TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    email_ids_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    mailbox TEXT
);
CREATE TABLE IF NOT EXISTS cloud_shares (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    emails_json TEXT NOT NULL,
    config_id TEXT NOT NULL,
    config_name TEXT NOT NULL,
    object_key TEXT NOT NULL,
    visibility TEXT NOT NULL,
    url TEXT NOT NULL,
    expires_at INTEGER,
    size INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS remotes (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    source_json TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_poll_at INTEGER,
    last_error TEXT
);
"#;

/// Indices are created after [`migrate`], since they may use columns that older
/// databases don't have yet.
const INDICES: &str = r#"
CREATE INDEX IF NOT EXISTS emails_mailbox ON emails(mailbox, received_at DESC);
CREATE INDEX IF NOT EXISTS emails_received ON emails(received_at);
CREATE UNIQUE INDEX IF NOT EXISTS emails_external ON emails(mailbox, external_id) WHERE external_id IS NOT NULL;
"#;

const SUMMARY_COLUMNS: &str = "id, mailbox, subject, from_addr, to_json, preview, received_at, size, read, has_attachments, spam_json, links_json";

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<EmailSummary> {
    let to_json: String = row.get(4)?;
    let spam_json: Option<String> = row.get(10)?;
    let links_json: Option<String> = row.get(11)?;
    let spam_score = spam_json
        .and_then(|json| serde_json::from_str::<SpamReport>(&json).ok())
        .map(|report| report.score);
    let broken_links = links_json
        .and_then(|json| serde_json::from_str::<Vec<LinkCheck>>(&json).ok())
        .map(|links| {
            links
                .iter()
                .filter(|link| matches!(link.status, crate::model::LinkStatus::Broken { .. }))
                .count() as u32
        });
    Ok(EmailSummary {
        id: row.get(0)?,
        mailbox: row.get(1)?,
        subject: row.get(2)?,
        from: row.get(3)?,
        to: serde_json::from_str(&to_json).unwrap_or_default(),
        preview: row.get(5)?,
        received_at: row.get(6)?,
        size: row.get::<_, i64>(7)? as u64,
        read: row.get(8)?,
        has_attachments: row.get(9)?,
        spam_score,
        broken_links,
    })
}

fn kind_str(kind: MailboxKind) -> &'static str {
    match kind {
        MailboxKind::Local => "local",
        MailboxKind::Remote => "remote",
    }
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )?;
        init(&conn).with_context(|| format!("upgrading database {}", path.display()))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        init(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // ---- mailboxes -------------------------------------------------------

    pub fn ensure_mailbox(&self, name: &str, kind: MailboxKind) -> anyhow::Result<bool> {
        let inserted = self.conn().execute(
            "INSERT OR IGNORE INTO mailboxes (name, kind, created_at) VALUES (?1, ?2, ?3)",
            params![name, kind_str(kind), chrono::Utc::now().timestamp()],
        )?;
        Ok(inserted > 0)
    }

    pub fn has_mailbox(&self, name: &str) -> anyhow::Result<bool> {
        Ok(self
            .conn()
            .prepare_cached("SELECT 1 FROM mailboxes WHERE name = ?1")?
            .exists([name])?)
    }

    pub fn mailboxes(&self) -> anyhow::Result<Vec<Mailbox>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT m.name, m.kind, m.created_at,
                    COUNT(e.id), COALESCE(SUM(CASE WHEN e.read = 0 THEN 1 ELSE 0 END), 0)
             FROM mailboxes m LEFT JOIN emails e ON e.mailbox = m.name
             GROUP BY m.name ORDER BY m.kind, m.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            let kind: String = row.get(1)?;
            Ok(Mailbox {
                name: row.get(0)?,
                kind: if kind == "remote" {
                    MailboxKind::Remote
                } else {
                    MailboxKind::Local
                },
                created_at: row.get(2)?,
                total: row.get::<_, i64>(3)? as u64,
                unread: row.get::<_, i64>(4)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Deletes the mailbox and every email inside it.
    pub fn delete_mailbox(&self, name: &str) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let ids = {
            let mut stmt = tx.prepare("SELECT id FROM emails WHERE mailbox = ?1")?;
            let ids = stmt.query_map([name], |row| row.get::<_, String>(0))?;
            ids.collect::<Result<Vec<_>, _>>()?
        };
        tx.execute("DELETE FROM emails WHERE mailbox = ?1", [name])?;
        tx.execute("DELETE FROM mailboxes WHERE name = ?1", [name])?;
        tx.commit()?;
        Ok(ids)
    }

    /// Deletes every email in the mailbox but keeps the mailbox itself.
    pub fn clear_mailbox(&self, name: &str) -> anyhow::Result<Vec<String>> {
        let conn = self.conn();
        let ids = {
            let mut stmt = conn.prepare("SELECT id FROM emails WHERE mailbox = ?1")?;
            let ids = stmt.query_map([name], |row| row.get::<_, String>(0))?;
            ids.collect::<Result<Vec<_>, _>>()?
        };
        conn.execute("DELETE FROM emails WHERE mailbox = ?1", [name])?;
        Ok(ids)
    }

    // ---- emails ----------------------------------------------------------

    /// Inserts an email. Returns `false` when an email with the same external id already exists.
    pub fn insert_email(&self, email: &NewEmail) -> anyhow::Result<bool> {
        let mut conn = self.conn();
        // One transaction, so a concurrent mailbox deletion can't remove the mailbox in between.
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO mailboxes (name, kind, created_at) VALUES (?1, ?2, ?3)",
            params![
                email.mailbox,
                kind_str(email.mailbox_kind),
                chrono::Utc::now().timestamp()
            ],
        )?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO emails (id, mailbox, subject, from_addr, to_json, preview, received_at,
                 envelope_from, envelope_to_json, size, has_attachments, raw, external_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                email.id,
                email.mailbox,
                email.subject,
                email.from,
                serde_json::to_string(&email.to)?,
                email.preview,
                email.received_at,
                email.envelope_from,
                serde_json::to_string(&email.envelope_to)?,
                email.raw.len() as i64,
                email.has_attachments,
                email.raw,
                email.external_id,
            ],
        )?;
        tx.commit()?;
        Ok(inserted > 0)
    }

    pub fn has_external_id(&self, mailbox: &str, external_id: &str) -> anyhow::Result<bool> {
        let found = self
            .conn()
            .query_row(
                "SELECT 1 FROM emails WHERE mailbox = ?1 AND external_id = ?2",
                params![mailbox, external_id],
                |_| Ok(()),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Lists emails, newest first. `search` matches subject, sender, recipients and preview.
    pub fn emails(
        &self,
        mailbox: Option<&str>,
        search: Option<&str>,
    ) -> anyhow::Result<Vec<EmailSummary>> {
        Ok(self.emails_page(mailbox, search, None, 0)?.0)
    }

    /// One page of emails, newest first, and the number of emails matching in total.
    pub fn emails_page(
        &self,
        mailbox: Option<&str>,
        search: Option<&str>,
        limit: Option<usize>,
        offset: usize,
    ) -> anyhow::Result<(Vec<EmailSummary>, usize)> {
        let conn = self.conn();
        let pattern = search
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("%{}%", escape_like(s)));
        const FILTER: &str = "WHERE (?1 IS NULL OR mailbox = ?1)
               AND (?2 IS NULL OR subject LIKE ?2 ESCAPE '\\' OR from_addr LIKE ?2 ESCAPE '\\'
                    OR to_json LIKE ?2 ESCAPE '\\' OR preview LIKE ?2 ESCAPE '\\')";
        // SQLite treats a negative LIMIT as "no limit".
        let limit = limit.map_or(-1, |l| i64::try_from(l).unwrap_or(i64::MAX));
        let offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let sql = format!(
            "SELECT {SUMMARY_COLUMNS} FROM emails {FILTER}
             ORDER BY received_at DESC, rowid DESC LIMIT ?3 OFFSET ?4"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![mailbox, pattern, limit, offset], summary_from_row)?;
        let emails: Vec<EmailSummary> = rows.collect::<Result<_, _>>()?;
        let total = if limit < 0 && offset == 0 {
            emails.len()
        } else {
            let count: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM emails {FILTER}"),
                params![mailbox, pattern],
                |row| row.get(0),
            )?;
            usize::try_from(count).unwrap_or_default()
        };
        Ok((emails, total))
    }

    /// Emails received after `since` (unix seconds), oldest first. Used by remote Scratchpad pulls.
    pub fn emails_since(&self, mailbox: &str, since: i64) -> anyhow::Result<Vec<EmailSummary>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {SUMMARY_COLUMNS} FROM emails WHERE mailbox = ?1 AND received_at >= ?2 ORDER BY received_at ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![mailbox, since], summary_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn summary(&self, id: &str) -> anyhow::Result<Option<EmailSummary>> {
        let conn = self.conn();
        let sql = format!("SELECT {SUMMARY_COLUMNS} FROM emails WHERE id = ?1");
        Ok(conn.query_row(&sql, [id], summary_from_row).optional()?)
    }

    pub fn raw(&self, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row("SELECT raw FROM emails WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn envelope(&self, id: &str) -> anyhow::Result<Option<(String, Vec<String>)>> {
        let row = self
            .conn()
            .query_row(
                "SELECT envelope_from, envelope_to_json FROM emails WHERE id = ?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        Ok(row.map(|(from, to)| (from, serde_json::from_str(&to).unwrap_or_default())))
    }

    pub fn analysis(
        &self,
        id: &str,
    ) -> anyhow::Result<(Option<Vec<LinkCheck>>, Option<SpamReport>)> {
        let row = self
            .conn()
            .query_row(
                "SELECT links_json, spam_json FROM emails WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        let Some((links, spam)) = row else {
            return Ok((None, None));
        };
        Ok((
            links.and_then(|json| serde_json::from_str(&json).ok()),
            spam.and_then(|json| serde_json::from_str(&json).ok()),
        ))
    }

    pub fn set_links(&self, id: &str, links: &[LinkCheck]) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE emails SET links_json = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(links)?],
        )?;
        Ok(())
    }

    pub fn set_spam(&self, id: &str, report: &SpamReport) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE emails SET spam_json = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(report)?],
        )?;
        Ok(())
    }

    pub fn set_read(&self, ids: &[String], read: bool) -> anyhow::Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut changed = 0;
        for id in ids {
            changed += tx.execute(
                "UPDATE emails SET read = ?2 WHERE id = ?1 AND read != ?2",
                params![id, read],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn delete_emails(&self, ids: &[String]) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut deleted = Vec::new();
        for id in ids {
            if tx.execute("DELETE FROM emails WHERE id = ?1", [id])? > 0 {
                deleted.push(id.clone());
            }
        }
        tx.commit()?;
        Ok(deleted)
    }

    pub fn delete_older_than(&self, cutoff: i64) -> anyhow::Result<Vec<String>> {
        let conn = self.conn();
        let ids = {
            let mut stmt = conn.prepare("SELECT id FROM emails WHERE received_at < ?1")?;
            let ids = stmt.query_map([cutoff], |row| row.get::<_, String>(0))?;
            ids.collect::<Result<Vec<_>, _>>()?
        };
        conn.execute("DELETE FROM emails WHERE received_at < ?1", [cutoff])?;
        Ok(ids)
    }

    // ---- shares ----------------------------------------------------------

    pub fn insert_share(&self, share: &Share) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO shares (token, title, email_ids_json, created_at, mailbox) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                share.token,
                share.title,
                serde_json::to_string(&share.email_ids)?,
                share.created_at,
                share.mailbox
            ],
        )?;
        Ok(())
    }

    pub fn shares(&self) -> anyhow::Result<Vec<Share>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT token, title, email_ids_json, created_at, mailbox FROM shares ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], share_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn share(&self, token: &str) -> anyhow::Result<Option<Share>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT token, title, email_ids_json, created_at, mailbox FROM shares WHERE token = ?1",
                [token],
                share_from_row,
            )
            .optional()?)
    }

    pub fn delete_share(&self, token: &str) -> anyhow::Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM shares WHERE token = ?1", [token])?
            > 0)
    }

    // ---- cloud shares ------------------------------------------------------

    pub fn upsert_cloud_share(&self, share: &CloudShare) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO cloud_shares (id, title, emails_json, config_id, config_name, \
             object_key, visibility, url, expires_at, size, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                share.id,
                share.title,
                serde_json::to_string(&share.emails)?,
                share.config_id,
                share.config_name,
                share.object_key,
                serde_json::to_string(&share.visibility)?,
                share.url,
                share.expires_at,
                share.size as i64,
                share.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn cloud_shares(&self) -> anyhow::Result<Vec<CloudShare>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{CLOUD_SHARE_SELECT} ORDER BY created_at DESC"))?;
        let rows = stmt.query_map([], cloud_share_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn cloud_share(&self, id: &str) -> anyhow::Result<Option<CloudShare>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{CLOUD_SHARE_SELECT} WHERE id = ?1"),
                [id],
                cloud_share_from_row,
            )
            .optional()?)
    }

    pub fn delete_cloud_share(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM cloud_shares WHERE id = ?1", [id])?
            > 0)
    }

    // ---- remote mailboxes -------------------------------------------------

    pub fn upsert_remote(&self, remote: &RemoteMailbox) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO remotes (id, name, source_json, enabled, last_poll_at, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, source_json = excluded.source_json,
                 enabled = excluded.enabled",
            params![
                remote.id,
                remote.name,
                serde_json::to_string(&remote.source)?,
                remote.enabled,
                remote.last_poll_at,
                remote.last_error
            ],
        )?;
        Ok(())
    }

    pub fn remotes(&self) -> anyhow::Result<Vec<RemoteMailbox>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, source_json, enabled, last_poll_at, last_error FROM remotes ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            let source: String = row.get(2)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                source,
                row.get::<_, bool>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut remotes = Vec::new();
        for row in rows {
            let (id, name, source, enabled, last_poll_at, last_error) = row?;
            // One unreadable row must not hide (and stop polling) every other remote.
            let source: RemoteSource = match serde_json::from_str(&source) {
                Ok(source) => source,
                Err(error) => {
                    tracing::warn!(remote = %name, %error, "skipping remote mailbox with invalid settings");
                    continue;
                }
            };
            remotes.push(RemoteMailbox {
                id,
                name,
                source,
                enabled,
                last_poll_at,
                last_error,
            });
        }
        Ok(remotes)
    }

    pub fn record_remote_poll(&self, id: &str, error: Option<&str>) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE remotes SET last_poll_at = ?2, last_error = ?3 WHERE id = ?1",
            params![id, chrono::Utc::now().timestamp(), error],
        )?;
        Ok(())
    }

    pub fn delete_remote(&self, id: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn();
        let name: Option<String> = conn
            .query_row("SELECT name FROM remotes WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        conn.execute("DELETE FROM remotes WHERE id = ?1", [id])?;
        Ok(name)
    }
}

fn share_from_row(row: &Row<'_>) -> rusqlite::Result<Share> {
    let ids: String = row.get(2)?;
    Ok(Share {
        token: row.get(0)?,
        title: row.get(1)?,
        email_ids: serde_json::from_str(&ids).unwrap_or_default(),
        created_at: row.get(3)?,
        mailbox: row.get(4)?,
    })
}

const CLOUD_SHARE_SELECT: &str =
    "SELECT id, title, emails_json, config_id, config_name, object_key, \
     visibility, url, expires_at, size, created_at FROM cloud_shares";

fn cloud_share_from_row(row: &Row<'_>) -> rusqlite::Result<CloudShare> {
    let emails: String = row.get(2)?;
    let visibility: String = row.get(6)?;
    Ok(CloudShare {
        id: row.get(0)?,
        title: row.get(1)?,
        emails: serde_json::from_str(&emails).unwrap_or_default(),
        config_id: row.get(3)?,
        config_name: row.get(4)?,
        object_key: row.get(5)?,
        visibility: serde_json::from_str(&visibility).unwrap_or_default(),
        url: row.get(7)?,
        expires_at: row.get(8)?,
        size: row.get::<_, i64>(9)? as u64,
        created_at: row.get(10)?,
    })
}

/// Escapes `%`, `_` and the escape character itself for `LIKE … ESCAPE '\'`.
fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn init(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(SCHEMA)?;
    migrate(conn)?;
    conn.execute_batch(INDICES)?;
    Ok(())
}

/// Columns added after the first release: `(table, column, definition)`.
const ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    ("emails", "read", "INTEGER NOT NULL DEFAULT 0"),
    ("emails", "links_json", "TEXT"),
    ("emails", "spam_json", "TEXT"),
    ("emails", "external_id", "TEXT"),
    ("shares", "mailbox", "TEXT"),
    ("remotes", "enabled", "INTEGER NOT NULL DEFAULT 1"),
    ("remotes", "last_poll_at", "INTEGER"),
    ("remotes", "last_error", "TEXT"),
    ("cloud_shares", "expires_at", "INTEGER"),
];

/// Brings databases created by older versions up to the current schema.
fn migrate(conn: &Connection) -> anyhow::Result<()> {
    for (table, column, definition) in ADDED_COLUMNS {
        let exists = conn
            .prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2")?
            .exists([table, column])?;
        if !exists {
            conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition};"
            ))?;
        }
    }
    Ok(())
}
