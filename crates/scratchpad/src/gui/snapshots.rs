//! Headless screenshots of the desktop UI.
//!
//!     cargo test -p scratchpad -- --ignored gui_snapshots
//!
//! Writes PNGs to `target/gui-snapshots/` (useful for docs and visual review).

use std::time::Duration;

use iced::Theme;
use scratchpad_core::{CredentialStorage, S3Config, S3Mode, Scratchpad, Settings, Visibility};
use scratchpad_web::export::{CloudShareRequest, ExportOptions};

use super::cloud::CloudMsg;
use super::preview::Device;
use super::{style, App, Message, Page, Tab};

/// Local MinIO used for the cloud share snapshots (skipped when it isn't running):
/// bucket `mails`, `email/shares/` publicly readable.
const MINIO: &str = "127.0.0.1:9000";

const INVOICE: &str = "From: Laravel App <noreply@app.test>\r\nTo: Jane Cooper <jane@example.com>\r\nCc: boss@example.com\r\nSubject: Your invoice #1042 is ready\r\nDate: Fri, 16 Oct 2026 21:40:00 +0000\r\nMessage-ID: <1042@app.test>\r\nX-HELO-View-File: ZW1haWxzLmludm9pY2U=\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nHi Jane,\r\n\r\nYour invoice #1042 for $49.00 is ready. Pay at https://example.invalid/pay/1042\r\n--b\r\nContent-Type: text/html\r\n\r\n<h1>Invoice #1042</h1><p>Hi Jane,</p><p>Your invoice for <b>$49.00</b> is ready.</p><p><a href=\"https://example.invalid/pay/1042\">Pay invoice</a></p><p>Thanks,<br>The Acme team</p>\r\n--b--\r\n";

/// Long, non-ASCII and right-to-left text, lots of recipients and an unbreakable URL.
fn stress_email() -> Vec<u8> {
    let to: Vec<String> = (1..=40).map(|i| format!("user{i}@example.com")).collect();
    let url = format!("https://example.invalid/{}", "a1b2c3d4".repeat(40));
    format!(
        "From: =?UTF-8?B?{name}?= <{local}@example.com>\r\nTo: {to}\r\nSubject: =?UTF-8?B?{subject}?=\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nمرحبا بالعالم — こんにちは 👋🏽 Ünïcödé\r\n{url}\r\n",
        name = b64("فريق الدعم الفني لشركة أكمي"),
        local = "a-very-long-local-part-for-testing-the-layout".repeat(2),
        to = to.join(", "),
        subject = b64(&format!(
            "{} — تأكيد الطلب رقم ٤٥٦ 🎉 {}",
            "This subject is very long and keeps going ".repeat(4),
            "Unbreakable".repeat(12)
        )),
    )
    .into_bytes()
}

fn b64(value: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in value.as_bytes().chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn email(from: &str, subject: &str, body: &str) -> Vec<u8> {
    format!("From: {from}\r\nTo: dev@example.com\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain\r\n\r\n{body}\r\n").into_bytes()
}

fn snapshot(app: &mut App, theme: &Theme, name: &str) {
    snapshot_sized(app, theme, name, 840.0);
}

/// Taller windows show the bottom of long pages.
fn snapshot_sized(app: &mut App, theme: &Theme, name: &str, height: f32) {
    snapshot_at(app, theme, name, (1320.0, height));
}

/// The smallest window the app allows.
const MIN: (f32, f32) = (960.0, 580.0);

/// Mac App Store screenshots: 1440x900 points, rendered at 2880x1800.
const STORE: (f32, f32) = (1440.0, 900.0);

fn snapshot_at(app: &mut App, theme: &Theme, name: &str, size: (f32, f32)) {
    app.window = iced::Size::new(size.0, size.1);
    app.list_height = 0.0;
    let mut simulator =
        iced_test::Simulator::with_size(super::view::app_settings(), size, app.view());
    let snapshot = simulator.snapshot(theme).expect("snapshot");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/gui-snapshots")
        .join(name);
    assert!(snapshot.matches_image(&path).expect("write snapshot"));
}

#[test]
#[ignore = "renders PNG screenshots; run explicitly"]
fn gui_snapshots() {
    let _ = std::fs::remove_dir_all(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/gui-snapshots"),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    let scratchpad = Scratchpad::in_memory(Settings {
        link_checker: false,
        spam_check: true,
        spamd_port: 1,
        ..Settings::default()
    })
    .unwrap();
    for (mailbox, raw) in [
        (
            "Acme",
            email(
                "GitHub <noreply@github.com>",
                "[acme/api] CI passed on main",
                "All checks have passed for commit 3f2c1a9.",
            ),
        ),
        (
            "Acme",
            email(
                "Stripe <receipts@stripe.com>",
                "Your receipt from Acme Inc. #2291-0042",
                "Amount paid $49.00. Thanks for your business.",
            ),
        ),
        (
            "Onboarding",
            email(
                "Acme <hello@acme.test>",
                "Welcome to Acme, Jane! 👋",
                "Let's get you set up in three quick steps.",
            ),
        ),
        (
            "Onboarding",
            email(
                "Acme Security <security@acme.test>",
                "Verify your email address",
                "Click the link below to verify your account.",
            ),
        ),
        ("Billing", INVOICE.as_bytes().to_vec()),
    ] {
        scratchpad
            .ingest_local(
                raw,
                mailbox,
                "noreply@app.test".into(),
                vec!["jane@example.com".into(), "audit@example.com".into()],
            )
            .unwrap();
    }
    std::thread::sleep(Duration::from_millis(300)); // background spam reports

    let (mut app, _) = App::new(scratchpad.clone(), runtime.handle().clone());
    app.status.smtp_running = true;
    let invoice = app.emails.first().unwrap().id.clone();
    let onboarding: Vec<String> = app
        .emails
        .iter()
        .filter(|e| e.mailbox == "Onboarding")
        .map(|e| e.id.clone())
        .collect();
    let dark = style::dark_theme();
    let light = style::light_theme();

    snapshot(&mut app, &dark, "01-empty-selection-dark");
    let _ = app.update(Message::SelectEmail(invoice.clone()));
    let _ = app.update(Message::ToggleCheck(app.emails[2].id.clone(), true));
    snapshot(&mut app, &dark, "02-inbox-dark");
    snapshot_at(&mut app, &dark, "store-01-inbox-dark", STORE);
    snapshot_at(&mut app, &dark, "min-02-inbox-dark", MIN);
    snapshot(&mut app, &light, "03-inbox-light");
    snapshot_at(&mut app, &light, "store-02-inbox-light", STORE);
    let _ = app.update(Message::Tab(Tab::Spam));
    snapshot(&mut app, &dark, "04-spam-dark");
    snapshot_at(&mut app, &dark, "store-04-spam-dark", STORE);
    let _ = app.update(Message::Tab(Tab::Headers));
    let _ = app.update(Message::ToggleMeta);
    snapshot(&mut app, &dark, "05-headers-dark");
    snapshot_at(&mut app, &light, "min-05-headers-light", MIN);
    let _ = app.update(Message::Page(Page::Setup));
    snapshot(&mut app, &dark, "06-setup-dark");
    snapshot_at(&mut app, &dark, "store-05-setup-dark", STORE);
    snapshot_at(&mut app, &dark, "min-06-setup-dark", MIN);
    let _ = app.update(Message::Page(Page::Settings));
    snapshot(&mut app, &dark, "07-settings-dark");
    snapshot(&mut app, &light, "08-settings-light");
    snapshot_at(&mut app, &light, "min-08-settings-light", MIN);

    // Multi-device previews.
    let _ = app.update(Message::SelectEmail(invoice.clone()));
    let _ = app.update(Message::Tab(Tab::Preview));
    let _ = app.update(Message::Device(Device::All));
    snapshot(&mut app, &dark, "09-preview-all-dark");
    snapshot_at(&mut app, &dark, "store-03-devices-dark", STORE);
    snapshot_at(&mut app, &dark, "min-09-preview-all-dark", MIN);
    let _ = app.update(Message::Device(Device::Mobile));
    snapshot(&mut app, &light, "10-preview-mobile-light");
    let _ = app.update(Message::Device(Device::Tablet));
    let _ = app.update(Message::Rotate);
    snapshot(&mut app, &dark, "11-preview-tablet-landscape-dark");
    let _ = app.update(Message::Rotate);

    // Cloud sharing: a local MinIO bucket and a synced one behind the vault.
    let minio_up =
        std::net::TcpStream::connect_timeout(&MINIO.parse().unwrap(), Duration::from_millis(300))
            .is_ok();
    let minio = runtime
        .block_on(scratchpad.save_s3_config(S3Config {
            id: String::new(),
            name: "MinIO (local)".into(),
            endpoint: format!("http://{MINIO}"),
            region: "us-east-1".into(),
            bucket: "mails".into(),
            access_key: "scratchpad".into(),
            secret_key: "scratchpad-secret".into(),
            cdn_domain: String::new(),
            mode: S3Mode::Both,
            path_style: true,
            storage: CredentialStorage::Local,
        }))
        .unwrap();
    runtime
        .block_on(scratchpad.vault_create("correct horse battery".into()))
        .unwrap();
    runtime
        .block_on(scratchpad.save_s3_config(S3Config {
            id: String::new(),
            name: "R2 production".into(),
            endpoint: "https://0123abcd.r2.cloudflarestorage.com".into(),
            region: "auto".into(),
            bucket: "scratchpad-shares".into(),
            access_key: "r2-access".into(),
            secret_key: "r2-secret".into(),
            cdn_domain: "share.example.com".into(),
            mode: S3Mode::Public,
            path_style: true,
            storage: CredentialStorage::Synced,
        }))
        .unwrap();
    let _ = app.update(Message::Resync);

    let _ = app.update(Message::ShareSelected);
    let _ = app.update(Message::Cloud(CloudMsg::Visibility(Visibility::Private)));
    snapshot(&mut app, &dark, "12-share-dialog-dark");
    snapshot_at(&mut app, &dark, "store-06-share-dark", STORE);
    snapshot(&mut app, &light, "13-share-dialog-light");
    snapshot_at(&mut app, &dark, "min-13-share-dialog-dark", MIN);
    let _ = app.update(Message::Cloud(CloudMsg::Cancel));

    let mut uploaded = vec![];
    if minio_up {
        for (title, ids, visibility) in [
            ("Invoice #1042", vec![invoice.clone()], Visibility::Public),
            ("Onboarding flow", onboarding.clone(), Visibility::Private),
        ] {
            let share = runtime
                .block_on(scratchpad_web::export::share_to_cloud(
                    &scratchpad,
                    CloudShareRequest {
                        title: title.into(),
                        ids,
                        config_id: minio.id.clone(),
                        visibility: Some(visibility),
                        expires_in_secs: Some(5 * 3600 + 120),
                        options: ExportOptions::default(),
                    },
                ))
                .expect("upload to MinIO");
            uploaded.push(share.id);
        }
        scratchpad
            .create_share("Welcome email", vec![onboarding[0].clone()])
            .unwrap();
        let _ = app.update(Message::Resync);
        let _ = app.update(Message::Page(Page::Shares));
        snapshot(&mut app, &dark, "14-shares-dark");
        snapshot(&mut app, &light, "15-shares-light");
    } else {
        eprintln!("MinIO isn't reachable at {MINIO}; skipping the cloud shares snapshot");
    }

    let _ = app.update(Message::Page(Page::Cloud));
    let _ = app.update(Message::Cloud(CloudMsg::EditConfig(minio.id.clone())));
    if minio_up {
        let checks = runtime.block_on(scratchpad.test_s3_config(minio.clone()));
        let _ = app.update(Message::Cloud(CloudMsg::Tested(String::new(), checks)));
    }
    snapshot_sized(&mut app, &dark, "16-cloud-form-dark", 1900.0);
    snapshot_at(&mut app, &light, "min-16-cloud-form-light", MIN);
    let _ = app.update(Message::Cloud(CloudMsg::CancelConfig));
    snapshot_sized(&mut app, &light, "17-cloud-light", 1100.0);
    snapshot_at(&mut app, &dark, "min-17-cloud-dark", MIN);
    let _ = app.update(Message::Cloud(CloudMsg::VaultOpen(
        super::forms::VaultAction::ChangePassword,
    )));
    snapshot_sized(&mut app, &dark, "18-vault-form-dark", 1100.0);
    let _ = app.update(Message::Cloud(CloudMsg::VaultCancel));

    // Remote mailbox form and the confirm bar at the minimum window size.
    let _ = app.update(Message::Page(Page::Remotes));
    let _ = app.update(Message::NewRemote);
    snapshot_at(&mut app, &light, "min-19-remote-form-light", MIN);
    let _ = app.update(Message::CancelRemote);

    // Long, right-to-left and emoji text.
    let stress = scratchpad
        .ingest_local(
            stress_email(),
            "A mailbox with a rather long name for the sidebar",
            "noreply@app.test".into(),
            vec!["user1@example.com".into()],
        )
        .unwrap();
    let _ = app.update(Message::Resync);
    let _ = app.update(Message::SelectMailbox(None));
    let _ = app.update(Message::SelectEmail(stress));
    let _ = app.update(Message::Ask(super::Confirm::ClearMailbox("Acme".into())));
    snapshot(&mut app, &dark, "20-long-text-dark");
    snapshot_at(&mut app, &light, "min-20-long-text-light", MIN);
    let _ = app.update(Message::ConfirmNo);
    let _ = app.update(Message::Device(Device::Mobile));
    snapshot_at(&mut app, &dark, "min-21-long-text-mobile-dark", MIN);

    for id in uploaded {
        let _ = runtime.block_on(scratchpad.delete_cloud_share(&id, false));
    }
}
