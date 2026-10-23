use std::time::Duration;

use scratchpad_core::{Scratchpad, Settings};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn start() -> std::sync::Arc<Scratchpad> {
    let settings = Settings {
        smtp_port: free_port(),
        link_checker: false,
        spam_check: true,
        spamd_port: 1, // force the built-in heuristics
        ..Settings::default()
    };
    let scratchpad = Scratchpad::in_memory(settings).unwrap();
    scratchpad.start();
    for _ in 0..50 {
        if scratchpad.status().smtp_running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    scratchpad
}

#[tokio::test]
async fn authenticated_mail_lands_in_named_mailbox() {
    let scratchpad = start().await;
    scratchpad.send_test_email("My Project").await.unwrap();

    let mailboxes = scratchpad.mailboxes().unwrap();
    assert_eq!(mailboxes.len(), 1);
    assert_eq!(mailboxes[0].name, "My Project");

    let emails = scratchpad.emails(Some("My Project"), None).unwrap();
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].subject, "Welcome to Scratchpad 👋");

    let detail = scratchpad.email(&emails[0].id).unwrap().unwrap();
    assert!(detail.html.unwrap().contains("Hello from Scratchpad"));
    assert_eq!(
        detail.envelope_to,
        vec!["developer@example.com".to_string()]
    );

    // the spam report runs in the background
    for _ in 0..100 {
        if scratchpad
            .email(&emails[0].id)
            .unwrap()
            .unwrap()
            .spam
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let spam = scratchpad
        .email(&emails[0].id)
        .unwrap()
        .unwrap()
        .spam
        .expect("spam report");
    assert_eq!(spam.engine, "builtin");

    assert_eq!(scratchpad.emails(None, Some("welcome")).unwrap().len(), 1);
    assert_eq!(
        scratchpad
            .emails(None, Some("nothing-matches"))
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn unauthenticated_mail_uses_default_mailbox_with_login_auth_and_dot_stuffing() {
    let scratchpad = start().await;
    let address = scratchpad.status().smtp_address;
    let stream = tokio::net::TcpStream::connect(&address).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let mut expect = async |prefix: &str| loop {
        let line = lines.next_line().await.unwrap().unwrap();
        if line.len() < 4 || line.as_bytes()[3] != b'-' {
            assert!(line.starts_with(prefix), "expected {prefix}, got {line}");
            return;
        }
    };
    expect("220").await;
    for (command, reply) in [
        ("EHLO test", "250"),
        ("MAIL FROM:<a@example.com>", "250"),
        ("RCPT TO:<b@example.com>", "250"),
        ("DATA", "354"),
    ] {
        write
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .unwrap();
        expect(reply).await;
    }
    write
        .write_all(b"Subject: plain\r\nFrom: a@example.com\r\n\r\n..leading dot\r\nbody\r\n.\r\n")
        .await
        .unwrap();
    expect("250").await;

    // AUTH LOGIN switches the mailbox for the next message
    write.write_all(b"AUTH LOGIN\r\n").await.unwrap();
    expect("334").await;
    write.write_all(b"VGVhbQ==\r\n").await.unwrap(); // "Team"
    expect("334").await;
    write.write_all(b"\r\n").await.unwrap();
    expect("235").await;
    for (command, reply) in [
        ("MAIL FROM:<a@example.com>", "250"),
        ("RCPT TO:<c@example.com>", "250"),
        ("DATA", "354"),
    ] {
        write
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .unwrap();
        expect(reply).await;
    }
    write
        .write_all(b"Subject: team\r\n\r\nhi\r\n.\r\nQUIT\r\n")
        .await
        .unwrap();
    expect("250").await;
    expect("221").await;

    let default = scratchpad.emails(Some("Default"), None).unwrap();
    assert_eq!(default.len(), 1);
    let raw = scratchpad.raw(&default[0].id).unwrap().unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("\r\n.leading dot\r\n"));
    assert_eq!(scratchpad.emails(Some("Team"), None).unwrap().len(), 1);
}
