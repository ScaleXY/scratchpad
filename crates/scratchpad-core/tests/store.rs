//! SQLite store: search, retention, deletion, dedupe and upgrading old databases.

use std::path::PathBuf;

use scratchpad_core::store::{NewEmail, Store};
use scratchpad_core::{MailboxKind, RemoteMailbox, RemoteSource, Scratchpad, Settings};

fn email(id: &str, mailbox: &str, subject: &str, received_at: i64) -> NewEmail {
    NewEmail {
        id: id.into(),
        mailbox: mailbox.into(),
        mailbox_kind: MailboxKind::Local,
        subject: subject.into(),
        from: "App <app@example.com>".into(),
        to: vec!["user@example.com".into()],
        preview: format!("preview of {subject}"),
        received_at,
        envelope_from: "app@example.com".into(),
        envelope_to: vec!["user@example.com".into()],
        has_attachments: false,
        raw: format!("Subject: {subject}\r\n\r\nbody").into_bytes(),
        external_id: None,
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("scratchpad-store-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn subjects(store: &Store, search: &str) -> Vec<String> {
    let mut found: Vec<String> = store
        .emails(None, Some(search))
        .unwrap()
        .into_iter()
        .map(|e| e.subject)
        .collect();
    found.sort();
    found
}

#[test]
fn search_treats_wildcards_literally() {
    let store = Store::open_in_memory().unwrap();
    for (i, subject) in [
        "100% off",
        "1000 off",
        "snake_case",
        "snakeXcase",
        r"C:\path",
        r"C:apath",
        "O'Brien \"quoted\"",
        "Grüße",
    ]
    .iter()
    .enumerate()
    {
        store
            .insert_email(&email(&i.to_string(), "Default", subject, i as i64))
            .unwrap();
    }
    assert_eq!(subjects(&store, "100%"), ["100% off"]);
    assert_eq!(subjects(&store, "snake_"), ["snake_case"]);
    assert_eq!(subjects(&store, r"C:\"), [r"C:\path"]);
    assert_eq!(subjects(&store, r"\"), [r"C:\path"]);
    assert_eq!(subjects(&store, "o'brien"), ["O'Brien \"quoted\""]);
    assert_eq!(subjects(&store, "Grüße"), ["Grüße"]);
    // SQL in the search term is just text.
    assert!(subjects(&store, "'; DROP TABLE emails; --").is_empty());
    assert_eq!(store.emails(None, Some("   ")).unwrap().len(), 8);
    assert_eq!(store.emails(None, None).unwrap().len(), 8);
    // Recipients and previews are searched too.
    assert_eq!(store.emails(None, Some("user@example")).unwrap().len(), 8);
    assert_eq!(subjects(&store, "preview of 1000"), ["1000 off"]);
}

#[test]
fn listing_order_mailboxes_and_counts() {
    let store = Store::open_in_memory().unwrap();
    store.insert_email(&email("a", "B box", "old", 10)).unwrap();
    store.insert_email(&email("b", "B box", "new", 20)).unwrap();
    store
        .insert_email(&email("c", "a box", "other", 15))
        .unwrap();
    let list = store.emails(Some("B box"), None).unwrap();
    assert_eq!(
        list.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    let since = store.emails_since("B box", 15).unwrap();
    assert_eq!(
        since.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["b"]
    );

    assert_eq!(
        store
            .set_read(&["a".into(), "missing".into()], true)
            .unwrap(),
        1
    );
    assert_eq!(store.set_read(&["a".into()], true).unwrap(), 0);
    let mailboxes = store.mailboxes().unwrap();
    let names: Vec<_> = mailboxes.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["a box", "B box"]);
    assert_eq!((mailboxes[1].total, mailboxes[1].unread), (2, 1));
    assert!(store.has_mailbox("a box").unwrap());
    assert!(!store.has_mailbox("A BOX").unwrap());

    assert_eq!(store.clear_mailbox("B box").unwrap().len(), 2);
    assert!(store.has_mailbox("B box").unwrap());
    assert_eq!(store.delete_mailbox("a box").unwrap(), ["c"]);
    assert!(!store.has_mailbox("a box").unwrap());
    assert!(store.summary("c").unwrap().is_none());
}

#[test]
fn retention_and_bulk_delete() {
    let store = Store::open_in_memory().unwrap();
    for (id, at) in [("old1", 100), ("old2", 199), ("edge", 200), ("new", 300)] {
        store.insert_email(&email(id, "Default", id, at)).unwrap();
    }
    let mut deleted = store.delete_older_than(200).unwrap();
    deleted.sort();
    assert_eq!(deleted, ["old1", "old2"]);
    assert!(store.delete_older_than(200).unwrap().is_empty());
    let deleted = store
        .delete_emails(&["edge".into(), "edge".into(), "nope".into()])
        .unwrap();
    assert_eq!(deleted, ["edge"]);
    assert_eq!(store.emails(None, None).unwrap().len(), 1);
}

#[test]
fn external_ids_dedupe_per_mailbox() {
    let store = Store::open_in_memory().unwrap();
    let mut first = email("1", "Remote", "x", 1);
    first.external_id = Some("imap:1:7".into());
    assert!(store.insert_email(&first).unwrap());
    let mut again = email("2", "Remote", "x", 2);
    again.external_id = Some("imap:1:7".into());
    assert!(!store.insert_email(&again).unwrap());
    let mut elsewhere = email("3", "Other", "x", 3);
    elsewhere.external_id = Some("imap:1:7".into());
    assert!(store.insert_email(&elsewhere).unwrap());
    assert!(store.has_external_id("Remote", "imap:1:7").unwrap());
    assert!(!store.has_external_id("Remote", "imap:1:8").unwrap());
}

#[test]
fn upgrades_databases_from_older_versions() {
    let dir = TempDir::new();
    let path = dir.0.join("scratchpad.sqlite3");
    {
        // The first schema: no read flag, analysis, external ids or share mailboxes.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE mailboxes (name TEXT PRIMARY KEY, kind TEXT NOT NULL, created_at INTEGER NOT NULL);
            CREATE TABLE emails (
                id TEXT PRIMARY KEY,
                mailbox TEXT NOT NULL REFERENCES mailboxes(name) ON DELETE CASCADE,
                subject TEXT NOT NULL, from_addr TEXT NOT NULL, to_json TEXT NOT NULL,
                preview TEXT NOT NULL, received_at INTEGER NOT NULL, envelope_from TEXT NOT NULL,
                envelope_to_json TEXT NOT NULL, size INTEGER NOT NULL,
                has_attachments INTEGER NOT NULL, raw BLOB NOT NULL
            );
            CREATE TABLE shares (token TEXT PRIMARY KEY, title TEXT NOT NULL,
                email_ids_json TEXT NOT NULL, created_at INTEGER NOT NULL);
            CREATE TABLE remotes (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, source_json TEXT NOT NULL);
            INSERT INTO mailboxes VALUES ('Default', 'local', 1);
            INSERT INTO emails VALUES ('old', 'Default', 'Old mail', 'a@b.c', '["d@e.f"]', 'p', 5,
                'a@b.c', '["d@e.f"]', 4, 0, X'41424344');
            INSERT INTO shares VALUES ('tok', 'Shared', '["old"]', 7);
            INSERT INTO remotes VALUES ('r1', 'Broken', 'not json');
            "#,
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let old = store.summary("old").unwrap().unwrap();
    assert_eq!(old.subject, "Old mail");
    assert!(!old.read);
    assert_eq!(store.raw("old").unwrap().unwrap(), b"ABCD");
    assert_eq!(store.analysis("old").unwrap(), (None, None));
    let share = store.share("tok").unwrap().unwrap();
    assert_eq!(
        (share.email_ids, share.mailbox),
        (vec!["old".to_string()], None)
    );
    // A remote with unreadable settings is skipped instead of failing the list.
    assert!(store.remotes().unwrap().is_empty());

    let mut remote_mail = email("new", "Remote", "x", 9);
    remote_mail.external_id = Some("scratchpad:1".into());
    assert!(store.insert_email(&remote_mail).unwrap());
    remote_mail.id = "dupe".into();
    assert!(!store.insert_email(&remote_mail).unwrap());
    drop(store);
    // Opening again is a no-op.
    let store = Store::open(&path).unwrap();
    assert_eq!(store.emails(None, None).unwrap().len(), 2);
}

#[test]
fn remote_names_must_be_unique() {
    let scratchpad = Scratchpad::in_memory(Settings::default()).unwrap();
    let remote = |name: &str| RemoteMailbox {
        id: String::new(),
        name: name.into(),
        source: RemoteSource::Scratchpad {
            base_url: "http://127.0.0.1:1".into(),
            username: None,
            password: None,
            mailbox: "Default".into(),
        },
        enabled: false,
        last_poll_at: None,
        last_error: None,
    };
    let saved = scratchpad.save_remote(remote(" Staging ")).unwrap();
    assert_eq!(saved.name, "Staging");
    let error = scratchpad.save_remote(remote("Staging")).unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
    assert!(scratchpad.save_remote(remote("  ")).is_err());
    // Updating the same remote keeps its name.
    scratchpad
        .save_remote(RemoteMailbox {
            enabled: false,
            ..saved.clone()
        })
        .unwrap();
    scratchpad.delete_remote(&saved.id, true).unwrap();
    assert!(scratchpad.remotes().unwrap().is_empty());
    assert!(scratchpad.mailboxes().unwrap().is_empty());
}

#[test]
fn reopening_a_data_dir_keeps_mail_and_recovers_corrupt_settings() {
    let dir = TempDir::new();
    std::fs::write(
        dir.0.join("settings.json"),
        b"{ \"smtp_port\": \"not a number\"",
    )
    .unwrap();
    let scratchpad = Scratchpad::open(&dir.0, |_| {}).unwrap();
    assert_eq!(scratchpad.settings(), Settings::default());
    // The broken file is kept for the user, not overwritten by the defaults.
    assert!(dir.0.join("settings.json.corrupt").exists());
    scratchpad
        .ingest_local(
            b"Subject: kept\r\n\r\nx".to_vec(),
            "Default",
            "a@b.c".into(),
            vec![],
        )
        .unwrap();
    scratchpad
        .update_settings(Settings {
            smtp_port: 2626,
            ..Settings::default()
        })
        .unwrap();
    drop(scratchpad);

    let scratchpad = Scratchpad::open(&dir.0, |s| s.web_port = 9999).unwrap();
    assert_eq!(scratchpad.settings().smtp_port, 2626);
    assert_eq!(scratchpad.settings().web_port, 9999);
    assert_eq!(scratchpad.emails(None, None).unwrap()[0].subject, "kept");
    // `customize` overrides aren't persisted.
    assert_eq!(Settings::load(&dir.0).unwrap().web_port, 8025);
}

#[test]
fn partial_and_unknown_settings_fields() {
    let dir = TempDir::new();
    std::fs::write(
        dir.0.join("settings.json"),
        br#"{ "smtp_port": 2600, "some_future_option": true, "forward": { "host": "smtp.example.com" } }"#,
    )
    .unwrap();
    let settings = Settings::load(&dir.0).unwrap();
    assert_eq!(settings.smtp_port, 2600);
    assert_eq!(settings.smtp_host, "127.0.0.1");
    assert_eq!(settings.forward.host, "smtp.example.com");
    assert_eq!(settings.forward.port, 587);
    assert!(Settings::load(&dir.0.join("missing")).unwrap() == Settings::default());

    settings.save(&dir.0).unwrap();
    assert_eq!(Settings::load(&dir.0).unwrap(), settings);
    assert!(!dir.0.join("settings.json.tmp").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.0.join("settings.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn in_memory_instances_clean_up_their_files() {
    let scratchpad = Scratchpad::in_memory(Settings::default()).unwrap();
    let dir = scratchpad.data_dir().to_path_buf();
    scratchpad.update_settings(Settings::default()).unwrap();
    assert!(dir.join("settings.json").exists());
    drop(scratchpad);
    assert!(!dir.exists());
}

#[test]
fn startup_overrides_are_not_saved() {
    let dir = std::env::temp_dir().join(format!("scratchpad-overrides-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let saved = scratchpad_core::Settings {
        web_port: 8025,
        spam_check: true,
        ..scratchpad_core::Settings::default()
    };
    saved.save(&dir).unwrap();

    // Like `scratchpad --web-port 9999 --web-auth admin:secret`.
    let scratchpad = scratchpad_core::Scratchpad::open(&dir, |s| {
        s.web_port = 9999;
        s.web_username = "admin".into();
        s.web_password = "secret".into();
    })
    .unwrap();
    assert_eq!(scratchpad.settings().web_port, 9999);

    // A UI saves an unrelated change and an explicit change to an overridden field.
    let mut update = scratchpad.settings();
    update.spam_check = false;
    update.web_username = "editor".into();
    scratchpad.update_settings(update).unwrap();

    let on_disk = scratchpad_core::Settings::load(&dir).unwrap();
    assert!(!on_disk.spam_check);
    assert_eq!(on_disk.web_port, 8025, "CLI override must not be persisted");
    assert_eq!(
        on_disk.web_password, "",
        "CLI password must not be persisted"
    );
    assert_eq!(on_disk.web_username, "editor", "explicit edits are saved");
    // The running instance keeps using the override.
    assert_eq!(scratchpad.settings().web_port, 9999);
    drop(scratchpad);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pages_are_cut_in_sqlite() {
    let store = Store::open_in_memory().unwrap();
    for i in 0..25 {
        let mailbox = if i % 5 == 0 { "Other" } else { "Main" };
        store
            .insert_email(&email(
                &format!("id{i:02}"),
                mailbox,
                &format!("Mail {i}"),
                i,
            ))
            .unwrap();
    }
    let ids = |page: &[scratchpad_core::EmailSummary]| {
        page.iter().map(|e| e.id.clone()).collect::<Vec<_>>()
    };

    // Newest first, with the total of all matches.
    let (page, total) = store.emails_page(None, None, Some(3), 0).unwrap();
    assert_eq!(total, 25);
    assert_eq!(ids(&page), ["id24", "id23", "id22"]);
    let (page, total) = store.emails_page(None, None, Some(3), 3).unwrap();
    assert_eq!(total, 25);
    assert_eq!(ids(&page), ["id21", "id20", "id19"]);

    // Mailbox and search filters apply to both the page and the total.
    let (page, total) = store.emails_page(Some("Main"), None, Some(2), 0).unwrap();
    assert_eq!(total, 20);
    assert_eq!(ids(&page), ["id24", "id23"]);
    let (page, total) = store.emails_page(None, Some("Mail 1"), Some(4), 1).unwrap();
    assert_eq!(total, 11, "Mail 1 and Mail 10-19");
    assert_eq!(page.len(), 4);

    // Past the end, and no limit at all.
    let (page, total) = store.emails_page(None, None, Some(10), 100).unwrap();
    assert!(page.is_empty());
    assert_eq!(total, 25);
    let (page, total) = store.emails_page(Some("Other"), None, None, 0).unwrap();
    assert_eq!((page.len(), total), (5, 5));
    assert_eq!(store.emails(None, None).unwrap().len(), 25);
}
