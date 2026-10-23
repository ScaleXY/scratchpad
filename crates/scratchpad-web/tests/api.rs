//! REST API routes, error shapes, security headers and the auth/CSRF/rebinding matrix.

mod common;

use base64::Engine;
use common::{header, ingest, serve};
use scratchpad_core::{RemoteMailbox, RemoteSource, Settings};
use serde_json::{json, Value};

const PLAIN: &str =
    "From: Ada <ada@example.com>\r\nTo: b@example.com\r\nSubject: Plain one\r\n\r\nHello there\r\n";

/// HTML with a `javascript:` link, a cid image and an attachment with a hostile file name.
const RICH: &str = "From: a@example.com\r\nTo: b@example.com\r\nSubject: =?UTF-8?Q?R=C3=A9sum=C3=A9_\"quoted\"?=\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/html\r\n\r\n<html><head></head><body><a href=\"javascript:alert(1)\">x</a><form action=\"/api/mailboxes/Default/clear\" method=\"post\"></form></body></html>\r\n--m\r\nContent-Type: text/html; name=\"evil.html\"\r\nContent-Disposition: attachment; filename*=UTF-8''r%C3%A9sum%C3%A9%22%3B%20x%3Dy.html\r\n\r\n<script>alert(1)</script>\r\n--m--\r\n";

fn view_data_email() -> String {
    let data = base64::engine::general_purpose::STANDARD
        .encode("<script>parent.document.title='pwned'</script><b>data</b>");
    format!("From: a@example.com\r\nSubject: View\r\nX-HELO-View: welcome\r\nX-HELO-View-Data: {data}\r\n\r\nhi\r\n")
}

async fn json(response: reqwest::Response) -> Value {
    response.json().await.unwrap()
}

#[tokio::test]
async fn email_routes() {
    let (scratchpad, url) = serve(Settings::default()).await;
    let client = reqwest::Client::new();
    let get = |path: &str| client.get(format!("{url}/api{path}")).send();
    let first = ingest(&scratchpad, PLAIN, "Team");
    let second = ingest(&scratchpad, PLAIN, "Team");
    let other = ingest(&scratchpad, RICH, "Other");

    let status = json(get("/status").await.unwrap()).await;
    assert!(status["version"].is_string());
    let mailboxes = json(get("/mailboxes").await.unwrap()).await;
    assert_eq!(mailboxes.as_array().unwrap().len(), 2);

    // Lists, search and paging (X-Total-Count has the full count).
    let all = get("/emails").await.unwrap();
    assert_eq!(header(&all, "x-total-count"), "3");
    assert_eq!(json(all).await.as_array().unwrap().len(), 3);
    let page = get("/emails?limit=1&offset=1").await.unwrap();
    assert_eq!(header(&page, "x-total-count"), "3");
    assert_eq!(json(page).await.as_array().unwrap().len(), 1);
    let team = json(get("/mailboxes/Team/emails?limit=10").await.unwrap()).await;
    assert_eq!(team.as_array().unwrap().len(), 2);
    let found = json(get("/emails?q=sum").await.unwrap()).await;
    assert_eq!(found[0]["id"], other.as_str());
    assert_eq!(
        json(get("/emails?mailbox=Other").await.unwrap()).await[0]["id"],
        other.as_str()
    );
    // Bad query values are JSON 400s.
    let bad = get("/emails?limit=lots").await.unwrap();
    assert_eq!(bad.status(), 400);
    assert!(json(bad).await["error"].is_string());

    // Detail and content.
    let detail = json(get(&format!("/emails/{first}")).await.unwrap()).await;
    assert_eq!(detail["summary"]["subject"], "Plain one");
    let missing = get("/emails/nope").await.unwrap();
    assert_eq!(missing.status(), 404);
    assert_eq!(json(missing).await["error"], "not found");
    let raw = get(&format!("/emails/{first}/raw")).await.unwrap();
    assert_eq!(header(&raw, "content-type"), "text/plain; charset=utf-8");
    assert!(header(&raw, "content-security-policy").contains("sandbox"));
    assert!(raw.text().await.unwrap().contains("Hello there"));
    // Plain-text emails are rendered as HTML too.
    let html = get(&format!("/emails/{first}/html")).await.unwrap();
    assert!(html.text().await.unwrap().contains("Hello there"));
    assert_eq!(
        get(&format!("/emails/{first}/view-data"))
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        get(&format!("/emails/{first}/attachments/0"))
            .await
            .unwrap()
            .status(),
        404
    );
    let bad_index = get(&format!("/emails/{first}/attachments/x"))
        .await
        .unwrap();
    assert_eq!(bad_index.status(), 400);
    assert!(json(bad_index).await["error"].is_string());

    // Read state.
    let read = client
        .post(format!("{url}/api/emails/read"))
        .json(&json!({ "ids": [first] }))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 204);
    assert!(scratchpad.summary(&first).unwrap().unwrap().read);
    client
        .post(format!("{url}/api/emails/read"))
        .json(&json!({ "ids": [first], "read": false }))
        .send()
        .await
        .unwrap();
    assert!(!scratchpad.summary(&first).unwrap().unwrap().read);

    // Forward and recheck need an existing email.
    let forward = |id: &str, to: Value| {
        client
            .post(format!("{url}/api/emails/{id}/forward"))
            .json(&json!({ "to": to }))
            .send()
    };
    assert_eq!(forward(&first, json!([" "])).await.unwrap().status(), 400);
    assert_eq!(
        forward("nope", json!(["x@example.com"]))
            .await
            .unwrap()
            .status(),
        404
    );
    let recheck = |id: String| client.post(format!("{url}/api/emails/{id}/recheck")).send();
    assert_eq!(recheck(first.clone()).await.unwrap().status(), 202);
    assert_eq!(recheck("nope".into()).await.unwrap().status(), 404);

    // Malformed JSON bodies get the JSON error shape too.
    let invalid = client
        .post(format!("{url}/api/emails/delete"))
        .header("content-type", "application/json")
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 400);
    assert!(json(invalid).await["error"].is_string());
    let unknown = get("/nope").await.unwrap();
    assert_eq!(unknown.status(), 404);
    assert!(json(unknown).await["error"].is_string());

    // Deleting.
    let deleted = client
        .delete(format!("{url}/api/emails/{first}"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);
    client
        .post(format!("{url}/api/emails/delete"))
        .json(&json!({ "ids": [second] }))
        .send()
        .await
        .unwrap();
    assert!(scratchpad.emails(Some("Team"), None).unwrap().is_empty());
    let cleared = client
        .post(format!("{url}/api/mailboxes/Other/clear"))
        .send()
        .await
        .unwrap();
    assert_eq!(cleared.status(), 204);
    assert!(scratchpad.emails(None, None).unwrap().is_empty());
    let removed = client
        .delete(format!("{url}/api/mailboxes/Other"))
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), 204);
    assert!(scratchpad
        .mailboxes()
        .unwrap()
        .iter()
        .all(|m| m.name != "Other"));
}

#[tokio::test]
async fn untrusted_content_is_sandboxed() {
    let (scratchpad, url) = serve(Settings::default()).await;
    let rich = ingest(&scratchpad, RICH, "Default");
    let view = ingest(&scratchpad, &view_data_email(), "Default");

    // Email HTML: no scripts, no forms, inert even when opened in its own tab.
    let html = reqwest::get(format!("{url}/api/emails/{rich}/html"))
        .await
        .unwrap();
    let csp = header(&html, "content-security-policy");
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("form-action 'none'"));
    assert!(csp.contains("sandbox allow-popups"));
    assert!(!csp.contains("allow-scripts") && !csp.contains("allow-forms"));
    assert_eq!(header(&html, "x-content-type-options"), "nosniff");
    assert_eq!(header(&html, "cache-control"), "no-store");

    // View data runs its own script, but never with the app's origin.
    let data = reqwest::get(format!("{url}/api/emails/{view}/view-data"))
        .await
        .unwrap();
    assert_eq!(data.status(), 200);
    let csp = header(&data, "content-security-policy");
    assert!(csp.contains("sandbox allow-scripts"));
    assert!(!csp.contains("allow-same-origin"));

    // An HTML attachment: sandboxed, and the hostile name can't break the header.
    let attachment = reqwest::get(format!("{url}/api/emails/{rich}/attachments/0"))
        .await
        .unwrap();
    assert_eq!(attachment.status(), 200);
    assert!(header(&attachment, "content-security-policy").ends_with("; sandbox"));
    assert_eq!(header(&attachment, "x-content-type-options"), "nosniff");
    let disposition = header(&attachment, "content-disposition");
    assert_eq!(
        disposition,
        "attachment; filename=\"r_sum___ x=y.html\"; filename*=UTF-8''r%C3%A9sum%C3%A9_%3B%20x%3Dy.html"
    );

    // The .eml download keeps the UTF-8 subject in filename*.
    let raw = reqwest::get(format!("{url}/api/emails/{rich}/raw?download=true"))
        .await
        .unwrap();
    assert_eq!(header(&raw, "content-type"), "message/rfc822");
    let disposition = header(&raw, "content-disposition");
    assert!(
        disposition.starts_with("attachment; filename=\"R_sum_ _quoted_.eml\"; filename*=UTF-8''R%C3%A9sum%C3%A9%20_quoted_.eml"),
        "{disposition}"
    );

    // Exports: the attachment-free viewer still can't be broken out of.
    let export = reqwest::get(format!("{url}/api/emails/{rich}/export"))
        .await
        .unwrap();
    assert!(header(&export, "content-disposition").contains("filename*=UTF-8''R%C3%A9sum%C3%A9"));
    let text = export.text().await.unwrap();
    assert!(text.contains("<meta http-equiv=\"Content-Security-Policy\""));
    assert!(!text.contains("<script>alert(1)</script>"));
}

#[tokio::test]
async fn app_shell_and_assets() {
    let (scratchpad, url) = serve(Settings::default()).await;
    let index = reqwest::get(format!("{url}/")).await.unwrap();
    let csp = header(&index, "content-security-policy");
    assert!(csp.contains("script-src 'self'") && !csp.contains("'unsafe-inline' 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert_eq!(header(&index, "referrer-policy"), "no-referrer");
    let body = index.text().await.unwrap();
    // No inline scripts, so the CSP needs no 'unsafe-inline' for scripts.
    assert!(!body.contains("<script>"));
    assert!(body.contains("<meta name=\"scratchpad-share\" content=\"\">"));

    for (path, kind) in [
        ("/assets/app.js", "text/javascript"),
        ("/assets/boot.js", "text/javascript"),
        ("/assets/devices.js", "text/javascript"),
        ("/assets/app.css", "text/css"),
        ("/assets/devices.css", "text/css"),
        ("/assets/icons.svg", "image/svg+xml"),
        ("/assets/icon.svg", "image/svg+xml"),
        ("/assets/fonts/inter-400.woff2", "font/woff2"),
    ] {
        let response = reqwest::get(format!("{url}{path}")).await.unwrap();
        assert_eq!(response.status(), 200, "{path}");
        assert!(
            header(&response, "content-type").starts_with(kind),
            "{path}"
        );
        assert_eq!(header(&response, "x-content-type-options"), "nosniff");
    }
    for path in [
        "/assets/fonts/..%2Fapp.js",
        "/assets/fonts/../../Cargo.toml",
        "/assets/fonts/inter-400.woff",
        "/assets/nope.js",
    ] {
        let response = reqwest::get(format!("{url}{path}")).await.unwrap();
        assert_eq!(response.status(), 404, "{path}");
    }

    // Share pages get the same shell and CSP, with the token.
    let id = ingest(&scratchpad, PLAIN, "Default");
    let share = scratchpad.create_share("S", vec![id]).unwrap();
    let page = reqwest::get(format!("{url}/s/{}", share.token))
        .await
        .unwrap();
    assert!(header(&page, "content-security-policy").contains("script-src 'self'"));
    assert!(page
        .text()
        .await
        .unwrap()
        .contains(&format!("content=\"{}\"", share.token)));
}

#[tokio::test]
async fn settings_passwords_are_write_only() {
    let (scratchpad, url) = serve(Settings {
        web_username: "admin".into(),
        web_password: "s3cret".into(),
        ..Settings::default()
    })
    .await;
    let mut forward = scratchpad.settings();
    forward.forward.password = "smtp-pass".into();
    scratchpad.update_settings(forward).unwrap();
    let client = reqwest::Client::new();
    let auth = |request: reqwest::RequestBuilder| request.basic_auth("admin", Some("s3cret"));

    let text = auth(client.get(format!("{url}/api/settings")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!text.contains("s3cret") && !text.contains("smtp-pass"));
    let mut settings: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(settings["web_password_set"], true);
    assert_eq!(settings["forward"]["password_set"], true);

    // Saving without passwords (null) keeps them.
    settings["web_password"] = Value::Null;
    settings["forward"]["password"] = Value::Null;
    settings["retention_days"] = 3.into();
    let saved = auth(client.put(format!("{url}/api/settings")))
        .json(&settings)
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    assert_eq!(json(saved).await["web_password"], "");
    let current = scratchpad.settings();
    assert_eq!(current.retention_days, 3);
    assert_eq!(current.web_password, "s3cret");
    assert_eq!(current.forward.password, "smtp-pass");

    // An explicit value replaces it.
    settings["forward"]["password"] = "".into();
    auth(client.put(format!("{url}/api/settings")))
        .json(&settings)
        .send()
        .await
        .unwrap();
    assert_eq!(scratchpad.settings().forward.password, "");

    // Validation.
    for (key, value) in [
        ("web_port", json!(0)),
        ("public_url", json!("scratchpad.example.com")),
        ("link_checker_timeout_secs", json!(0)),
        ("smtp_port", json!(70000)),
        ("smtp_host", json!(" ")),
    ] {
        let mut invalid = settings.clone();
        invalid[key] = value;
        let response = auth(client.put(format!("{url}/api/settings")))
            .json(&invalid)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{key}");
        assert!(json(response).await["error"].is_string());
    }
    assert_eq!(scratchpad.settings().web_port, Settings::default().web_port);
}

#[tokio::test]
async fn remotes_and_shares() {
    // Port 1: nothing listens there, so the test email fails.
    let (scratchpad, url) = serve(Settings {
        smtp_port: 1,
        ..Settings::default()
    })
    .await;
    let client = reqwest::Client::new();
    let remote = scratchpad
        .save_remote(RemoteMailbox {
            id: String::new(),
            name: "Staging".into(),
            source: RemoteSource::Imap {
                host: "imap.example.com".into(),
                port: 993,
                tls: true,
                username: "u".into(),
                password: "imap-pass".into(),
                folder: "INBOX".into(),
                delete_after_fetch: false,
            },
            enabled: false,
            last_poll_at: None,
            last_error: None,
        })
        .unwrap();

    let listed = client
        .get(format!("{url}/api/remotes"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!listed.contains("imap-pass"));
    let mut body: Value = serde_json::from_str::<Value>(&listed).unwrap()[0].clone();
    assert_eq!(body["source"]["has_password"], true);

    // Editing without a password keeps it.
    body["enabled"] = true.into();
    let updated = client
        .put(format!("{url}/api/remotes/{}", remote.id))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), 200);
    assert!(!updated.text().await.unwrap().contains("imap-pass"));
    let stored = scratchpad.remotes().unwrap().remove(0);
    assert!(stored.enabled);
    assert!(
        matches!(stored.source, RemoteSource::Imap { ref password, .. } if password == "imap-pass")
    );
    assert_eq!(
        client
            .put(format!("{url}/api/remotes/nope"))
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let created = client
        .post(format!("{url}/api/remotes"))
        .json(&json!({ "id": "", "name": "Other", "enabled": false, "source": {
            "type": "scratchpad", "base_url": "http://127.0.0.1:1", "mailbox": "Default", "password": "pw" },
            "last_poll_at": null, "last_error": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let created = json(created).await;
    assert_eq!(created["source"]["password"], Value::Null);
    assert_eq!(created["source"]["has_password"], true);
    let nameless = client
        .post(format!("{url}/api/remotes"))
        .json(
            &json!({ "id": "", "name": " ", "enabled": false, "source": {
            "type": "scratchpad", "base_url": "http://x", "mailbox": "Default" },
            "last_poll_at": null, "last_error": null }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(nameless.status(), 400);
    let deleted = client
        .delete(format!(
            "{url}/api/remotes/{}?delete_emails=true",
            remote.id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);
    assert_eq!(scratchpad.remotes().unwrap().len(), 1);
    assert_eq!(
        client
            .post(format!("{url}/api/remotes/poll"))
            .send()
            .await
            .unwrap()
            .status(),
        202
    );

    // Server shares.
    let id = ingest(&scratchpad, PLAIN, "Default");
    let unknown = client
        .post(format!("{url}/api/shares"))
        .json(&json!({ "title": "x", "ids": ["nope"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);
    let empty = client
        .post(format!("{url}/api/shares"))
        .json(&json!({ "title": "x", "ids": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), 400);
    let created = json(
        client
            .post(format!("{url}/api/shares"))
            .json(&json!({ "title": "Mine", "ids": [id] }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let token = created["share"]["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 32);
    assert_eq!(created["url"], format!("{url}/s/{token}"));
    let listed = json(
        client
            .get(format!("{url}/api/shares"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(listed[0]["share"]["token"], token.as_str());
    let shared = json(
        client
            .get(format!("{url}/s/{token}/api/share?limit=1"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(shared["total"], 1);
    assert_eq!(
        client
            .delete(format!("{url}/api/shares/{token}"))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert!(scratchpad.shares().unwrap().is_empty());

    // Test email needs a running SMTP server; the error is a JSON 400.
    let failed = client
        .post(format!("{url}/api/test-email"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(failed.status(), 400);
    assert!(json(failed).await["error"].is_string());
}

#[tokio::test]
async fn share_pages_only_expose_shared_emails() {
    let (scratchpad, url) = serve(Settings::default()).await;
    let shared = ingest(&scratchpad, &view_data_email(), "Team");
    let private = ingest(&scratchpad, RICH, "Team");
    let token = scratchpad
        .create_share("S", vec![shared.clone()])
        .unwrap()
        .token;
    let status = |path: String| async move { reqwest::get(path).await.unwrap().status() };
    let base = format!("{url}/s/{token}/api/emails");
    for suffix in ["", "/raw", "/raw?download=true", "/html", "/view-data"] {
        assert_eq!(
            status(format!("{base}/{shared}{suffix}")).await,
            200,
            "{suffix}"
        );
        assert_eq!(
            status(format!("{base}/{private}{suffix}")).await,
            404,
            "{suffix}"
        );
    }
    assert_eq!(status(format!("{base}/{private}/attachments/0")).await, 404);
    // Unknown tokens look exactly like missing emails.
    assert_eq!(
        status(format!("{url}/s/{}/api/share", "x".repeat(32))).await,
        404
    );
    assert_eq!(status(format!("{url}/s/{}", "x".repeat(32))).await, 404);
    // Share responses are not cached, and view data is sandboxed here too.
    let data = reqwest::get(format!("{base}/{shared}/view-data"))
        .await
        .unwrap();
    assert_eq!(header(&data, "cache-control"), "no-store");
    assert!(header(&data, "content-security-policy").contains("sandbox allow-scripts"));
    // The API under a share is read-only.
    let client = reqwest::Client::new();
    let post = client
        .post(format!("{base}/{shared}"))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), 405);
}

#[tokio::test]
async fn auth_csrf_and_rebinding_matrix() {
    let (scratchpad, url) = serve(Settings {
        web_username: "admin".into(),
        web_password: "pw".into(),
        public_url: "https://scratchpad.example.com".into(),
        ..Settings::default()
    })
    .await;
    let host = url.trim_start_matches("http://").to_string();
    let client = reqwest::Client::new();

    // Missing or wrong credentials: 401 with a challenge (JSON for the API).
    let anonymous = client
        .get(format!("{url}/api/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    assert!(header(&anonymous, "www-authenticate").starts_with("Basic realm=\"Scratchpad\""));
    assert!(json(anonymous).await["error"].is_string());
    let events = client
        .get(format!("{url}/api/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), 401);
    for (user, pass) in [("admin", "nope"), ("nope", "pw"), ("admin", "")] {
        let response = client
            .get(format!("{url}/api/status"))
            .basic_auth(user, Some(pass))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401, "{user}:{pass}");
    }
    // The scheme is case-insensitive.
    let encoded = base64::engine::general_purpose::STANDARD.encode("admin:pw");
    let lowercase = client
        .get(format!("{url}/api/status"))
        .header("Authorization", format!("basic {encoded}"))
        .send()
        .await
        .unwrap();
    assert_eq!(lowercase.status(), 200);

    let post = |origin: Option<&str>, fetch_site: Option<&str>| {
        let mut request = client
            .post(format!("{url}/api/remotes/poll"))
            .basic_auth("admin", Some("pw"));
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        if let Some(site) = fetch_site {
            request = request.header("Sec-Fetch-Site", site);
        }
        async move { request.send().await.unwrap().status() }
    };
    // Valid credentials don't make a cross-site request acceptable.
    assert_eq!(post(Some("https://evil.example"), None).await, 403);
    assert_eq!(post(Some("null"), None).await, 403);
    assert_eq!(post(None, Some("cross-site")).await, 403);
    assert_eq!(
        post(Some(&format!("http://{host}")), Some("same-origin")).await,
        202
    );
    // Behind a proxy that rewrites Host, the public URL's origin is accepted.
    assert_eq!(
        post(Some("https://scratchpad.example.com"), None).await,
        202
    );
    assert_eq!(post(None, None).await, 202);
    // Safe methods aren't blocked by Origin (CORS keeps responses unreadable).
    let get = client
        .get(format!("{url}/api/status"))
        .basic_auth("admin", Some("pw"))
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 200);
    assert!(get.headers().get("access-control-allow-origin").is_none());

    // Share pages and assets stay public.
    let id = ingest(&scratchpad, PLAIN, "Default");
    let token = scratchpad.create_share("S", vec![id]).unwrap().token;
    let page = client.get(format!("{url}/s/{token}")).send().await.unwrap();
    assert_eq!(page.status(), 200);

    // Without auth on a loopback server, foreign Host names (DNS rebinding) are refused,
    // for the app, the API and the event stream; share pages are public anyway.
    let (_scratchpad, open) = serve(Settings::default()).await;
    for path in ["/", "/api/status", "/api/events"] {
        let rebound = client
            .get(format!("{open}{path}"))
            .header("Host", "evil.example:80")
            .send()
            .await
            .unwrap();
        assert_eq!(rebound.status(), 403, "{path}");
    }
    let local = client
        .get(format!("{open}/api/status"))
        .header("Host", "127.0.0.1")
        .send()
        .await
        .unwrap();
    assert_eq!(local.status(), 200);
}

#[tokio::test]
async fn events_stream_new_mail() {
    let (scratchpad, url) = serve(Settings::default()).await;
    let mut response = reqwest::get(format!("{url}/api/events")).await.unwrap();
    assert_eq!(header(&response, "content-type"), "text/event-stream");
    assert_eq!(header(&response, "cache-control"), "no-cache");
    ingest(&scratchpad, PLAIN, "Default");
    let mut received = String::new();
    while !received.contains("email_received") {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), response.chunk())
            .await
            .expect("an event")
            .unwrap()
            .expect("an open stream");
        received.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(received.contains("Plain one"));
}

#[tokio::test]
async fn export_and_cloud_errors() {
    let (scratchpad, url) = serve(Settings::default()).await;
    let client = reqwest::Client::new();
    let id = ingest(&scratchpad, PLAIN, "Default");
    let other = ingest(&scratchpad, RICH, "Default");

    let empty = client
        .get(format!("{url}/api/export"))
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), 400);
    assert!(json(empty).await["error"].is_string());
    let both = client
        .post(format!("{url}/api/export"))
        .json(&json!({ "ids": [id, other], "title": "Two \"emails\"\r\nX: y" }))
        .send()
        .await
        .unwrap();
    assert_eq!(both.status(), 200);
    assert_eq!(
        header(&both, "content-disposition"),
        "attachment; filename=\"Two _emails___X_ y.html\"; filename*=UTF-8''Two%20_emails___X_%20y.html"
    );
    assert_eq!(header(&both, "cache-control"), "no-store");

    // Cloud shares need a known config and existing emails.
    let unknown_email = client
        .post(format!("{url}/api/cloud/shares"))
        .json(&json!({ "ids": ["nope"], "config_id": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown_email.status(), 404);
    let unknown_config = client
        .post(format!("{url}/api/cloud/shares"))
        .json(&json!({ "ids": [id], "config_id": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown_config.status(), 400);
    assert!(json(unknown_config).await["error"].is_string());
    let shares = json(
        client
            .get(format!("{url}/api/cloud/shares"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(shares, json!([]));
    let renew = client
        .post(format!("{url}/api/cloud/shares/nope/renew"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(renew.status(), 400);
    let missing_field = client
        .post(format!("{url}/api/cloud/vault/unlock"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_field.status(), 422);
    assert!(json(missing_field).await["error"].is_string());
}
