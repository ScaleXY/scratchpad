mod common;

use common::serve;
use scratchpad_core::{Scratchpad, Settings};

const EMAIL: &str = "From: a@example.com\r\nTo: b@example.com\r\nSubject: Hi\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<html><head></head><body><img src=\"cid:logo\"></body></html>\r\n--x\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--x--\r\n";

#[tokio::test]
async fn basic_auth_protects_api_but_not_shares() {
    let (scratchpad, url) = serve(Settings {
        web_username: "admin".into(),
        web_password: "s3cret".into(),
        ..Settings::default()
    })
    .await;
    let client = reqwest::Client::new();

    assert_eq!(
        client
            .get(format!("{url}/api/mailboxes"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client.get(format!("{url}/")).send().await.unwrap().status(),
        401
    );
    let ok = client
        .get(format!("{url}/api/mailboxes"))
        .basic_auth("admin", Some("s3cret"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let wrong = client
        .get(format!("{url}/api/mailboxes"))
        .basic_auth("admin", Some("nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
    assert_eq!(
        client
            .get(format!("{url}/healthz"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{url}/assets/app.js"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    let shared = scratchpad
        .ingest_local(
            EMAIL.as_bytes().to_vec(),
            "Team",
            "a@example.com".into(),
            vec!["b@example.com".into()],
        )
        .unwrap();
    let private = scratchpad
        .ingest_local(
            EMAIL.as_bytes().to_vec(),
            "Team",
            "a@example.com".into(),
            vec!["b@example.com".into()],
        )
        .unwrap();
    let share = scratchpad
        .create_share("Demo", vec![shared.clone()])
        .unwrap();
    assert_eq!(
        scratchpad.share_url(&share.token).unwrap(),
        format!("{url}/s/{}", share.token)
    );

    let page = client
        .get(format!("{url}/s/{}", share.token))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.text().await.unwrap().contains(&share.token));
    let listing: serde_json::Value = client
        .get(format!("{url}/s/{}/api/share", share.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listing["emails"].as_array().unwrap().len(), 1);
    assert_eq!(
        client
            .get(format!("{url}/s/{}/api/emails/{shared}", share.token))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    // emails outside the share stay private
    assert_eq!(
        client
            .get(format!("{url}/s/{}/api/emails/{private}", share.token))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .get(format!("{url}/s/not-a-token"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );

    scratchpad.delete_share(&share.token).unwrap();
    assert_eq!(
        client
            .get(format!("{url}/s/{}/api/share", share.token))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}

#[tokio::test]
async fn inbox_shares_follow_the_mailbox_live() {
    let (scratchpad, url) = serve(Settings::default()).await;
    let ingest = |mailbox: &str| {
        scratchpad
            .ingest_local(
                EMAIL.as_bytes().to_vec(),
                mailbox,
                "a@example.com".into(),
                vec!["b@example.com".into()],
            )
            .unwrap()
    };
    let first = ingest("Team");
    let other = ingest("Private");
    let client = reqwest::Client::new();

    let created: serde_json::Value = client
        .post(format!("{url}/api/shares"))
        .json(&serde_json::json!({ "title": "Team inbox", "mailbox": "Team" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = created["share"]["token"].as_str().unwrap().to_string();
    assert_eq!(created["share"]["mailbox"], "Team");

    // An email arriving after the link was created shows up and is readable.
    let later = ingest("Team");
    let listing: serde_json::Value = client
        .get(format!("{url}/s/{token}/api/share"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = listing["emails"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&first.as_str()) && ids.contains(&later.as_str()));
    let status = |id: String| {
        let client = client.clone();
        let url = format!("{url}/s/{token}/api/emails/{id}");
        async move { client.get(url).send().await.unwrap().status() }
    };
    assert_eq!(status(later.clone()).await, 200);
    // Other mailboxes stay private.
    assert_eq!(status(other).await, 404);

    // Unknown mailboxes are rejected.
    let missing = client
        .post(format!("{url}/api/shares"))
        .json(&serde_json::json!({ "title": "x", "mailbox": "Nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 400);
}

#[tokio::test]
async fn export_and_cloud_api() {
    let (scratchpad, url) = serve(Settings::default()).await;
    let id = scratchpad
        .ingest_local(EMAIL.as_bytes().to_vec(), "Default", String::new(), vec![])
        .unwrap();
    let client = reqwest::Client::new();

    let export = client
        .get(format!("{url}/api/emails/{id}/export"))
        .send()
        .await
        .unwrap();
    assert_eq!(export.status(), 200);
    assert_eq!(
        export.headers()["content-disposition"],
        "attachment; filename=\"Hi.html\"; filename*=UTF-8''Hi.html"
    );
    let html = export.text().await.unwrap();
    assert!(html.contains("scratchpad-data") && !html.contains("/assets/"));
    let missing = client
        .get(format!("{url}/api/export?ids={id},nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    // Secrets are write-only.
    let config = serde_json::json!({
        "name": "Bucket", "endpoint": "https://s3.example.com", "bucket": "b",
        "access_key": "AKID", "secret_key": "super-secret", "mode": "both",
    });
    let saved: serde_json::Value = client
        .post(format!("{url}/api/cloud/configs"))
        .json(&config)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(saved["has_secret"], true);
    assert_eq!(saved["secret_key"], "");
    let overview = client
        .get(format!("{url}/api/cloud"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(overview.contains("Bucket") && !overview.contains("super-secret"));

    // Synced configs need an unlocked vault.
    let synced = serde_json::json!({ "name": "Synced", "storage": "synced", "endpoint": "https://s3.example.com",
        "bucket": "b", "access_key": "AKID", "secret_key": "s" });
    let post = |path: &str, body: serde_json::Value| {
        let request = client.post(format!("{url}/api/{path}")).json(&body);
        async move { request.send().await.unwrap().status() }
    };
    assert_eq!(post("cloud/configs", synced.clone()).await, 400);
    assert_eq!(
        post(
            "cloud/vault/create",
            serde_json::json!({ "password": "correct horse" })
        )
        .await,
        204
    );
    assert_eq!(post("cloud/configs", synced).await, 200);
    assert_eq!(post("cloud/vault/lock", serde_json::json!({})).await, 204);
    assert_eq!(scratchpad.s3_configs().len(), 1);
    assert_eq!(
        post(
            "cloud/vault/unlock",
            serde_json::json!({ "password": "wrong horse" })
        )
        .await,
        400
    );
    assert_eq!(
        post(
            "cloud/vault/unlock",
            serde_json::json!({ "password": "correct horse" })
        )
        .await,
        204
    );
    assert_eq!(scratchpad.s3_configs().len(), 2);
}

#[tokio::test]
async fn html_is_sandboxed_and_cid_images_resolve() {
    let (scratchpad, url) = serve(Settings::default()).await;
    let id = scratchpad
        .ingest_local(EMAIL.as_bytes().to_vec(), "Default", String::new(), vec![])
        .unwrap();
    let response = reqwest::get(format!("{url}/api/emails/{id}/html"))
        .await
        .unwrap();
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(csp.contains("default-src 'none'"));
    let html = response.text().await.unwrap();
    assert!(html.contains("<head><base target=\"_blank\">"));
    assert!(html.contains("src=\"attachments/0\""));

    let image = reqwest::get(format!("{url}/api/emails/{id}/attachments/0"))
        .await
        .unwrap();
    assert_eq!(image.headers()["content-type"], "image/png");
    assert_eq!(image.bytes().await.unwrap().len(), 8);

    let raw = reqwest::get(format!("{url}/api/emails/{id}/raw?download=true"))
        .await
        .unwrap();
    assert!(raw.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .contains("Hi.eml"));
}

#[tokio::test]
async fn remote_mailbox_pulls_from_another_instance() {
    let (server, url) = serve(Settings {
        web_username: "ci".into(),
        web_password: "pw".into(),
        ..Settings::default()
    })
    .await;
    server
        .ingest_local(
            EMAIL.as_bytes().to_vec(),
            "Staging",
            "a@example.com".into(),
            vec!["b@example.com".into()],
        )
        .unwrap();

    let desktop = Scratchpad::in_memory(Settings {
        link_checker: false,
        spam_check: false,
        ..Settings::default()
    })
    .unwrap();
    let remote = desktop
        .save_remote(scratchpad_core::RemoteMailbox {
            id: String::new(),
            name: "Staging server".into(),
            source: scratchpad_core::RemoteSource::Scratchpad {
                base_url: url,
                username: Some("ci".into()),
                password: Some("pw".into()),
                mailbox: "Staging".into(),
            },
            enabled: true,
            last_poll_at: None,
            last_error: None,
        })
        .unwrap();

    assert_eq!(
        scratchpad_core::remote::poll(&desktop, &remote)
            .await
            .unwrap(),
        1
    );
    // polling again must not import duplicates
    assert_eq!(
        scratchpad_core::remote::poll(&desktop, &remote)
            .await
            .unwrap(),
        0
    );
    let emails = desktop.emails(Some("Staging server"), None).unwrap();
    assert_eq!(emails.len(), 1);
    let detail = desktop.email(&emails[0].id).unwrap().unwrap();
    assert_eq!(detail.envelope_to, vec!["b@example.com".to_string()]);
    assert_eq!(
        desktop.mailboxes().unwrap()[0].kind,
        scratchpad_core::MailboxKind::Remote
    );
}

#[tokio::test]
async fn rejects_csrf_and_dns_rebinding() {
    let (_scratchpad, url) = serve(Settings::default()).await;
    let client = reqwest::Client::new();
    // cross-site form post
    let csrf = client
        .post(format!("{url}/api/mailboxes/Default/clear"))
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(csrf.status(), 403);
    // same-origin request from the web app
    let host = url.trim_start_matches("http://");
    let same = client
        .post(format!("{url}/api/remotes/poll"))
        .header("Origin", format!("http://{host}"))
        .send()
        .await
        .unwrap();
    assert_eq!(same.status(), 202);
    // a rebound DNS name pointing at 127.0.0.1
    let rebinding = client
        .get(format!("{url}/api/emails"))
        .header("Host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(rebinding.status(), 403);
    let local = client
        .get(format!("{url}/api/emails"))
        .header("Host", "localhost:1234")
        .send()
        .await
        .unwrap();
    assert_eq!(local.status(), 200);
}
