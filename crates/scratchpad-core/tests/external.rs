//! Tests against real external services. They are `#[ignore]`d; run them with
//! `cargo test -p scratchpad-core --test external -- --ignored` once the services are up.
//! Each test skips (passes with a message) when its environment variables are missing.
//!
//! * S3 (e.g. MinIO): `SCRATCHPAD_TEST_S3_ENDPOINT` (`http://127.0.0.1:19000`),
//!   `SCRATCHPAD_TEST_S3_BUCKET`, `SCRATCHPAD_TEST_S3_ACCESS_KEY`, `SCRATCHPAD_TEST_S3_SECRET_KEY`.
//!   The bucket's `email/shares/` prefix must be publicly readable, everything else private.
//! * IMAP + SMTP relay (GreenMail with `-Dgreenmail.auth.disabled`): `SCRATCHPAD_TEST_IMAP_HOST`,
//!   `SCRATCHPAD_TEST_IMAP_PORT`, `SCRATCHPAD_TEST_SMTP_RELAY_HOST`, `SCRATCHPAD_TEST_SMTP_RELAY_PORT`. The relay
//!   delivers to per-recipient mailboxes; the IMAP login is the recipient address with any password.
//! * SpamAssassin: `SCRATCHPAD_TEST_SPAMD_HOST`, `SCRATCHPAD_TEST_SPAMD_PORT`.

use std::sync::Arc;
use std::time::Duration;

use scratchpad_core::{
    CredentialStorage, ForwardEncryption, ForwardSettings, PublishRequest, RemoteMailbox,
    RemoteSource, S3Config, S3Mode, Scratchpad, Settings, Visibility,
};

fn env(names: &[&str]) -> Option<Vec<String>> {
    let values: Option<Vec<String>> = names
        .iter()
        .map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
        .collect();
    if values.is_none() {
        eprintln!("skipping: set {} to run this test", names.join(", "));
    }
    values
}

fn scratchpad() -> Arc<Scratchpad> {
    Scratchpad::in_memory(Settings {
        link_checker: false,
        spam_check: false,
        ..Settings::default()
    })
    .unwrap()
}

// ---- S3 -------------------------------------------------------------------------

fn s3_config() -> Option<S3Config> {
    let v = env(&[
        "SCRATCHPAD_TEST_S3_ENDPOINT",
        "SCRATCHPAD_TEST_S3_BUCKET",
        "SCRATCHPAD_TEST_S3_ACCESS_KEY",
        "SCRATCHPAD_TEST_S3_SECRET_KEY",
    ])?;
    Some(S3Config {
        id: String::new(),
        name: "Test bucket".into(),
        endpoint: v[0].clone(),
        region: "us-east-1".into(),
        bucket: v[1].clone(),
        access_key: v[2].clone(),
        secret_key: v[3].clone(),
        cdn_domain: String::new(),
        mode: S3Mode::Both,
        path_style: true,
        storage: CredentialStorage::Local,
    })
}

async fn get(url: &str) -> (u16, Vec<u8>) {
    let response = reqwest::get(url).await.unwrap();
    let status = response.status().as_u16();
    (status, response.bytes().await.unwrap().to_vec())
}

#[tokio::test]
#[ignore = "needs an S3 service (SCRATCHPAD_TEST_S3_*)"]
async fn s3_connection_test_passes() {
    let Some(config) = s3_config() else { return };
    let scratchpad = scratchpad();
    let checks = scratchpad.test_s3_config(config.clone()).await;
    assert!(checks.iter().all(|c| c.ok), "{checks:#?}");
    assert!(checks
        .iter()
        .any(|c| c.name == "Private objects stay private"));

    // Wrong credentials fail with the provider's error, without leaking the signature.
    let checks = scratchpad
        .test_s3_config(S3Config {
            secret_key: "wrong-secret".into(),
            ..config
        })
        .await;
    let upload = checks
        .iter()
        .find(|c| c.name.starts_with("Upload"))
        .unwrap();
    assert!(!upload.ok);
    assert!(
        upload.detail.contains("SignatureDoesNotMatch"),
        "{}",
        upload.detail
    );
    assert!(!upload.detail.contains("X-Amz-Signature"));
}

#[tokio::test]
#[ignore = "needs an S3 service (SCRATCHPAD_TEST_S3_*)"]
async fn s3_public_and_private_shares() {
    let Some(config) = s3_config() else { return };
    scratchpad_core::tls::install_crypto_provider();
    let scratchpad = scratchpad();
    let config = scratchpad.save_s3_config(config).await.unwrap();
    let id = scratchpad
        .ingest_local(
            b"Subject: Shared mail\r\n\r\nx".to_vec(),
            "Default",
            String::new(),
            vec![],
        )
        .unwrap();
    let html = b"<!doctype html><title>share</title>hello".to_vec();
    let request = |visibility| PublishRequest {
        title: "Share".into(),
        email_ids: vec![id.clone()],
        config_id: config.id.clone(),
        visibility: Some(visibility),
        expires_in_secs: Some(600),
    };

    let public = scratchpad
        .publish_cloud_share(request(Visibility::Public), html.clone())
        .await
        .unwrap();
    assert!(public.object_key.starts_with("email/shares/"));
    assert_eq!(public.emails[0].subject, "Shared mail");
    assert_eq!(public.size, html.len() as u64);
    assert_eq!(get(&public.url).await, (200, html.clone()));

    let private = scratchpad
        .publish_cloud_share(request(Visibility::Private), html.clone())
        .await
        .unwrap();
    assert!(private.object_key.starts_with("email/private/"));
    assert!(private.expires_at.is_some());
    assert_eq!(get(&private.url).await, (200, html.clone()));
    let direct = config.direct_url(&private.object_key).unwrap();
    assert_ne!(
        get(direct.as_str()).await.0,
        200,
        "private object is public"
    );
    let renewed = scratchpad
        .renew_cloud_share(&private.id, Some(120))
        .unwrap();
    assert_ne!(renewed.url, private.url);
    assert_eq!(get(&renewed.url).await.0, 200);
    assert_eq!(scratchpad.cloud_shares().unwrap().len(), 2);

    scratchpad
        .delete_cloud_share(&public.id, false)
        .await
        .unwrap();
    scratchpad
        .delete_cloud_share(&private.id, false)
        .await
        .unwrap();
    assert_ne!(get(&public.url).await.0, 200);
    assert_ne!(get(&renewed.url).await.0, 200);
    assert!(scratchpad.cloud_shares().unwrap().is_empty());
}

// ---- IMAP, SMTP relay and forwarding ------------------------------------------------

struct Mail {
    imap_host: String,
    imap_port: u16,
    relay_host: String,
    relay_port: u16,
}

fn mail() -> Option<Mail> {
    let v = env(&[
        "SCRATCHPAD_TEST_IMAP_HOST",
        "SCRATCHPAD_TEST_IMAP_PORT",
        "SCRATCHPAD_TEST_SMTP_RELAY_HOST",
        "SCRATCHPAD_TEST_SMTP_RELAY_PORT",
    ])?;
    Some(Mail {
        imap_host: v[0].clone(),
        imap_port: v[1].parse().expect("SCRATCHPAD_TEST_IMAP_PORT"),
        relay_host: v[2].clone(),
        relay_port: v[3].parse().expect("SCRATCHPAD_TEST_SMTP_RELAY_PORT"),
    })
}

impl Mail {
    fn forward_settings(&self) -> ForwardSettings {
        ForwardSettings {
            host: self.relay_host.clone(),
            port: self.relay_port,
            username: String::new(),
            password: String::new(),
            encryption: ForwardEncryption::None,
            from_override: String::new(),
        }
    }

    fn remote(&self, name: &str, user: &str, delete_after_fetch: bool) -> RemoteMailbox {
        RemoteMailbox {
            id: String::new(),
            name: name.into(),
            source: RemoteSource::Imap {
                host: self.imap_host.clone(),
                port: self.imap_port,
                tls: false,
                username: user.into(),
                password: "any password".into(),
                folder: "INBOX".into(),
                delete_after_fetch,
            },
            enabled: true,
            last_poll_at: None,
            last_error: None,
        }
    }

    async fn send(&self, to: &str, subject: &str) {
        let raw = format!(
            "From: sender@example.com\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{}@example.com>\r\n\r\nbody of {subject}\r\n",
            uuid::Uuid::new_v4()
        );
        scratchpad_core::forward::forward(
            &self.forward_settings(),
            raw.as_bytes(),
            "sender@example.com",
            &[to.to_string()],
        )
        .await
        .unwrap();
    }
}

async fn poll_until(
    scratchpad: &Arc<Scratchpad>,
    remote: &RemoteMailbox,
    expected: usize,
) -> usize {
    let mut total = 0;
    for _ in 0..50 {
        total += scratchpad_core::remote::poll(scratchpad, remote)
            .await
            .unwrap();
        if total >= expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    total
}

#[tokio::test]
#[ignore = "needs GreenMail (SCRATCHPAD_TEST_IMAP_*, SCRATCHPAD_TEST_SMTP_RELAY_*)"]
async fn imap_remote_mailbox_imports_once_and_deletes() {
    let Some(mail) = mail() else { return };
    let scratchpad = scratchpad();
    let user = format!("imap-{}@example.com", uuid::Uuid::new_v4().simple());
    for i in 0..3 {
        mail.send(&user, &format!("imap {i}")).await;
    }

    let keep = mail.remote("Staging", &user, false);
    assert_eq!(poll_until(&scratchpad, &keep, 3).await, 3);
    // Already imported messages are not imported again.
    assert_eq!(
        scratchpad_core::remote::poll(&scratchpad, &keep)
            .await
            .unwrap(),
        0
    );
    let mut subjects: Vec<_> = scratchpad
        .emails(Some("Staging"), None)
        .unwrap()
        .into_iter()
        .map(|e| e.subject)
        .collect();
    subjects.sort();
    assert_eq!(subjects, ["imap 0", "imap 1", "imap 2"]);

    // Another remote with delete-after-fetch imports them too, then removes them.
    let drain = mail.remote("Drain", &user, true);
    assert_eq!(
        scratchpad_core::remote::poll(&scratchpad, &drain)
            .await
            .unwrap(),
        3
    );
    let check = mail.remote("Check", &user, false);
    assert_eq!(
        scratchpad_core::remote::poll(&scratchpad, &check)
            .await
            .unwrap(),
        0
    );
    assert_eq!(scratchpad.emails(Some("Drain"), None).unwrap().len(), 3);
}

#[tokio::test]
#[ignore = "needs GreenMail (SCRATCHPAD_TEST_IMAP_*, SCRATCHPAD_TEST_SMTP_RELAY_*)"]
async fn imap_errors_are_reported() {
    let Some(mail) = mail() else { return };
    let scratchpad = scratchpad();
    let mut remote = mail.remote("Missing folder", "someone@example.com", false);
    if let RemoteSource::Imap { folder, .. } = &mut remote.source {
        *folder = "Does-Not-Exist".into();
    }
    assert!(scratchpad_core::remote::poll(&scratchpad, &remote)
        .await
        .is_err());
    let mut remote = mail.remote("Closed port", "someone@example.com", false);
    if let RemoteSource::Imap { port, .. } = &mut remote.source {
        *port = 1;
    }
    assert!(scratchpad_core::remote::poll(&scratchpad, &remote)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "needs GreenMail (SCRATCHPAD_TEST_IMAP_*, SCRATCHPAD_TEST_SMTP_RELAY_*)"]
async fn forwarding_through_a_relay() {
    let Some(mail) = mail() else { return };
    let scratchpad = scratchpad();
    scratchpad
        .update_settings(Settings {
            forward: mail.forward_settings(),
            ..scratchpad.settings()
        })
        .unwrap();
    let id = scratchpad
        .ingest_local(
            b"From: app@example.com\r\nTo: dev@example.com\r\nSubject: Forward me\r\n\r\nhello\r\n"
                .to_vec(),
            "Default",
            "app@example.com".into(),
            vec!["dev@example.com".into()],
        )
        .unwrap();
    let user = format!("fwd-{}@example.com", uuid::Uuid::new_v4().simple());
    assert!(scratchpad
        .forward(&id, &["not an address".into()])
        .await
        .is_err());
    scratchpad
        .forward(&id, std::slice::from_ref(&user))
        .await
        .unwrap();

    let remote = mail.remote("Forwarded", &user, false);
    assert_eq!(poll_until(&scratchpad, &remote, 1).await, 1);
    assert_eq!(
        scratchpad.emails(Some("Forwarded"), None).unwrap()[0].subject,
        "Forward me"
    );
}

// ---- SpamAssassin --------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs spamd (SCRATCHPAD_TEST_SPAMD_*)"]
async fn spamd_report() {
    let Some(v) = env(&["SCRATCHPAD_TEST_SPAMD_HOST", "SCRATCHPAD_TEST_SPAMD_PORT"]) else {
        return;
    };
    let port: u16 = v[1].parse().expect("SCRATCHPAD_TEST_SPAMD_PORT");
    let gtube =
        b"Subject: Test spam mail (GTUBE)\r\nFrom: a@example.com\r\nTo: b@example.com\r\n\r\n\
        XJS*C4JDBQADN1.NSBN3*2IDNEN*GTUBE-STANDARD-ANTI-UBE-TEST-EMAIL*C.34X\r\n";
    let report = scratchpad_core::spam::analyze(gtube, &v[0], port).await;
    assert_eq!(report.engine, "spamassassin", "{:?}", report.note);
    assert!(report.is_spam);
    assert!(report.score >= 100.0, "{}", report.score);
    assert!(
        report.rules.iter().any(|r| r.name == "GTUBE"),
        "{:?}",
        report.rules
    );

    let clean =
        b"Subject: Hello\r\nFrom: a@example.com\r\nTo: b@example.com\r\n\r\nJust saying hi.\r\n";
    let report = scratchpad_core::spam::analyze(clean, &v[0], port).await;
    assert_eq!(report.engine, "spamassassin");
    assert!(!report.is_spam);
}
