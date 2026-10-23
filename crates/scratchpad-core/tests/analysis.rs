//! MIME parsing, link extraction and the built-in spam heuristics.

use base64::Engine;
use scratchpad_core::{links, parse, spam, EmailSummary, LinkStatus};

fn summary() -> EmailSummary {
    EmailSummary {
        id: "1".into(),
        mailbox: "Default".into(),
        subject: String::new(),
        from: String::new(),
        to: vec![],
        preview: String::new(),
        received_at: 0,
        size: 0,
        read: false,
        has_attachments: false,
        spam_score: None,
        broken_links: None,
    }
}

fn detail(raw: &[u8]) -> scratchpad_core::EmailDetail {
    parse::detail(raw, summary(), String::new(), vec![])
}

#[test]
fn headers_only_and_empty_messages() {
    let o = parse::overview(b"Subject: only headers\r\n");
    assert_eq!(o.subject, "only headers");
    assert_eq!(o.preview, "");
    let d = detail(b"Subject: only headers\r\n");
    assert!(d.html.is_none());

    let o = parse::overview(b"");
    assert!(!o.subject.is_empty());
    let d = detail(b"");
    assert!(d.attachments.is_empty());
    let o = parse::overview(b"\x00\xff\xfe garbage without headers");
    assert!(!o.subject.is_empty());
    assert!(parse::attachment(b"", 0).is_none());
}

#[test]
fn nested_multiparts_and_attachment_indices() {
    let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG fake image");
    let raw = format!(
        "From: App <app@example.com>\r\nTo: a@example.com, B <b@example.com>\r\nCc: c@example.com\r\n\
         Subject: Nested\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n\
         --outer\r\nContent-Type: multipart/related; boundary=rel\r\n\r\n\
         --rel\r\nContent-Type: multipart/alternative; boundary=alt\r\n\r\n\
         --alt\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain body\r\n\
         --alt\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
         <p>Hi <img src=\"cid:img1\"> <img src=\"CID:img10\"> <img src='cid:img%2B1'> <a href=\"cid:unknown\">x</a></p>\r\n\
         --alt--\r\n\
         --rel\r\nContent-Type: image/png\r\nContent-ID: <img1>\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
         --rel\r\nContent-Type: image/png\r\nContent-ID: <img10>\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
         --rel\r\nContent-Type: image/png\r\nContent-ID: <img+1>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
         --rel--\r\n\
         --outer\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n\
         --outer\r\nContent-Type: message/rfc822\r\n\r\nSubject: inner\r\n\r\ninner body\r\n\
         --outer--\r\n"
    );
    let raw = raw.as_bytes();
    let o = parse::overview(raw);
    assert_eq!(o.preview, "Plain body");
    assert!(o.has_attachments);

    let d = parse::detail(
        raw,
        summary(),
        String::new(),
        vec!["hidden@example.com".into(), "B@example.com".into()],
    );
    assert_eq!(d.to.len(), 2);
    assert_eq!(d.cc.len(), 1);
    // Envelope recipients missing from To/Cc are Bcc (case-insensitively).
    assert_eq!(d.bcc.len(), 1);
    assert_eq!(d.bcc[0].email, "hidden@example.com");
    let names: Vec<_> = d.attachments.iter().map(|a| (a.index, a.inline)).collect();
    assert_eq!(
        names,
        [(0, true), (1, true), (2, true), (3, false), (4, false)]
    );
    assert_eq!(d.attachments[3].filename, "report.pdf");
    for attachment in &d.attachments {
        let (meta, bytes) = parse::attachment(raw, attachment.index).unwrap();
        assert_eq!(&meta, attachment);
        assert!(!bytes.is_empty());
    }
    assert_eq!(parse::attachment(raw, 0).unwrap().1, b"\x89PNG fake image");
    assert_eq!(parse::attachment(raw, 3).unwrap().1, b"%PDF-1.4");
    assert!(String::from_utf8_lossy(&parse::attachment(raw, 4).unwrap().1).contains("inner body"));
    assert!(parse::attachment(raw, 5).is_none());

    // Each cid is rewritten as a whole: `cid:img1` must not clobber `cid:img10`.
    let html = parse::html_with_cids(raw, |index| format!("/att/{index}")).unwrap();
    assert!(html.contains("src=\"/att/0\""), "{html}");
    assert!(html.contains("src=\"/att/1\""), "{html}");
    assert!(html.contains("src='/att/2'"), "{html}");
    assert!(html.contains("href=\"cid:unknown\""), "{html}");
}

#[test]
fn broken_transfer_encodings_do_not_panic() {
    let raw = b"Subject: broken\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\n!!!not*base64###\r\n\
        --b\r\nContent-Type: text/html\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n=ZZ<b>bold=\r\n\
        --b\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\nAAA\r\n\
        --b"; // missing closing boundary
    let o = parse::overview(raw);
    assert_eq!(o.subject, "broken");
    let d = detail(raw);
    for attachment in &d.attachments {
        parse::attachment(raw, attachment.index);
    }
    let _ = parse::html_with_cids(raw, |i| i.to_string());
    spam::builtin(raw);
}

#[test]
fn non_utf8_charsets_are_decoded() {
    let latin1 = b"Subject: =?ISO-8859-1?Q?Gr=FC=DFe?=\r\nContent-Type: text/plain; charset=iso-8859-1\r\n\r\nSch\xf6ne Gr\xfc\xdfe\r\n";
    assert_eq!(parse::overview(latin1).subject, "Grüße");
    assert_eq!(parse::overview(latin1).preview, "Schöne Grüße");

    let sjis = b"Subject: =?Shift_JIS?B?g26DjYFbgUWDj4Fbg4uDaA==?=\r\nContent-Type: text/plain; charset=Shift_JIS\r\n\r\n\x83n\x83\x8d\x81[\x81E\x83\x8f\x81[\x83\x8b\x83h\r\n";
    assert_eq!(parse::overview(sjis).subject, "ハロー・ワールド");
    assert_eq!(parse::overview(sjis).preview, "ハロー・ワールド");

    let iso2022 = b"Subject: test\r\nContent-Type: text/plain; charset=ISO-2022-JP\r\n\r\n\x1b$B%O%m!<!&%o!<%k%I\x1b(B\r\n";
    assert_eq!(parse::overview(iso2022).preview, "ハロー・ワールド");

    let gbk =
        b"Subject: x\r\nContent-Type: text/html; charset=gbk\r\n\r\n<p>\xc4\xe3\xba\xc3</p>\r\n";
    assert!(detail(gbk).html.unwrap().contains("你好"));

    // Invalid UTF-8 declared as UTF-8 is replaced, not rejected.
    let bad = b"Subject: x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nok \xff\xfe end\r\n";
    assert!(parse::overview(bad).preview.starts_with("ok"));
}

#[test]
fn huge_messages_are_summarised_quickly() {
    let mut raw = b"Subject: huge\r\nContent-Type: text/html\r\n\r\n".to_vec();
    for i in 0..200_000 {
        raw.extend_from_slice(
            format!("<p><a href=\"https://example.com/{i}\">link {i}</a></p>\n").as_bytes(),
        );
    }
    let started = std::time::Instant::now();
    let o = parse::overview(&raw);
    assert!(o.preview.chars().count() <= 160);
    let html = parse::parse(&raw)
        .unwrap()
        .body_html(0)
        .unwrap()
        .into_owned();
    let found = links::extract(Some(&html), None);
    assert_eq!(found.len(), links::MAX_LINKS);
    assert_eq!(found[0].1.as_deref(), Some("link 0"));
    spam::builtin(&raw);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn link_extraction() {
    let html = r##"
        <A HREF="https://a.test/?q=1&amp;r=2">First <b>link</b></A>
        <a href = 'https://b.test/'>quoted</a>
        <a href=https://c.test/x>unquoted</a>
        <a href="https://a.test/?q=1&amp;r=2">duplicate</a>
        <a href="#top">anchor</a>
        <a href="mailto:x@y.z">mail</a>
        <a href="tel:+123">call</a>
        <a href="{{ url }}">template</a>
        <a href="https://d.test/%7B%7Bid%7D%7D">encoded template</a>
        <a href="/relative">relative</a>
        <a href="https://e.test/&amp;lt;">double encoded</a>
        <link hreflang="en" href="https://f.test/style.css">
        <a href="unterminated
    "##;
    let found = links::extract(
        Some(html),
        Some("Text with https://g.test/page. and (https://h.test/x)"),
    );
    let urls: Vec<&str> = found.iter().map(|(u, _)| u.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://a.test/?q=1&r=2",
            "https://b.test/",
            "https://c.test/x",
            "#top",
            "mailto:x@y.z",
            "tel:+123",
            "{{ url }}",
            "https://d.test/%7B%7Bid%7D%7D",
            "/relative",
            "https://e.test/&lt;",
            "https://f.test/style.css",
            "https://g.test/page",
            "https://h.test/x",
        ]
    );
    assert_eq!(found[0].1.as_deref(), Some("First link"));
    let checks = links::pending(&found);
    let state = |url: &str| &checks.iter().find(|c| c.url == url).unwrap().status;
    assert_eq!(*state("https://b.test/"), LinkStatus::Pending);
    assert!(matches!(state("#top"), LinkStatus::Skipped { .. }));
    assert!(matches!(state("mailto:x@y.z"), LinkStatus::Skipped { .. }));
    assert!(matches!(state("{{ url }}"), LinkStatus::Broken { .. }));
    assert!(matches!(state("/relative"), LinkStatus::Broken { .. }));
    assert!(links::extract(None, None).is_empty());
    // Multi-byte text around links doesn't trip slicing.
    let found = links::extract(Some("ü<a href=\"https://ü.test\">ü</a>ü"), None);
    assert_eq!(found[0].1.as_deref(), Some("ü"));
}

#[tokio::test]
async fn link_checker_against_a_local_server() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    scratchpad_core::tls::install_crypto_provider();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let response = if request.starts_with("HEAD /no-head") {
                    "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\n\r\n".to_string()
                } else if request.contains(" /missing ") {
                    "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_string()
                } else if request.contains(" /slow ") {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    "HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n".to_string()
                } else {
                    // A huge body: the checker must not download it.
                    "HTTP/1.1 200 OK\r\ncontent-length: 1000000000\r\n\r\n".to_string()
                };
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    let urls = ["/ok", "/no-head", "/missing", "/slow"].map(|p| (format!("{base}{p}"), None));
    let client = reqwest::Client::new();
    let started = std::time::Instant::now();
    let checked = links::check_all(
        &client,
        links::pending(&urls),
        std::time::Duration::from_millis(500),
    )
    .await;
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    assert!(matches!(&checked[0].status, LinkStatus::Ok { method, .. } if method == "HEAD"));
    assert!(matches!(&checked[1].status, LinkStatus::Ok { method, .. } if method == "GET"));
    assert!(matches!(
        checked[2].status,
        LinkStatus::Broken {
            status: Some(404),
            ..
        }
    ));
    assert!(matches!(&checked[3].status, LinkStatus::Broken { error, .. } if error == "timed out"));
}

fn rules(raw: &[u8]) -> Vec<String> {
    spam::builtin(raw)
        .rules
        .into_iter()
        .map(|r| r.name)
        .collect()
}

#[test]
fn builtin_spam_rules() {
    let clean = b"From: App <app@example.com>\r\nTo: u@example.com\r\nSubject: Your invoice\r\n\
        Date: Mon, 1 Jan 2024 00:00:00 +0000\r\nMessage-ID: <1@example.com>\r\nList-Unsubscribe: <mailto:u@example.com>\r\n\r\nHello, your invoice is attached.\r\n";
    let report = spam::builtin(clean);
    assert!(report.rules.is_empty(), "{:?}", report.rules);
    assert_eq!(
        (report.score, report.is_spam, report.engine.as_str()),
        (0.0, false, "builtin")
    );

    let spammy = b"From: \"winner@bank.test\" <x@spam.test>\r\nSubject: FREE MONEY NOW!!!\r\n\
        MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Type: text/html\r\n\r\n<form><script>x</script><img src=x><a href=\"http://192.168.1.1/x\">click here</a> bit.ly/abc 100% free act now</form>\r\n\
        --b\r\nContent-Type: application/octet-stream; name=\"setup.EXE\"\r\nContent-Disposition: attachment; filename=\"setup.EXE\"\r\n\r\nMZ\r\n--b--\r\n";
    let names = rules(spammy);
    for expected in [
        "SUBJ_ALL_CAPS",
        "SUBJ_EXCESS_EXCLAIM",
        "MISSING_DATE",
        "MISSING_MID",
        "FROM_NAME_ADDR_MISMATCH",
        "MISSING_HEADERS",
        "MIME_HTML_ONLY",
        "HTML_IMAGE_RATIO",
        "HTML_SCRIPT",
        "HTML_FORM",
        "URL_SHORTENER",
        "NUMERIC_HTTP_ADDR",
        "SPAMMY_PHRASES",
        "NO_LIST_UNSUBSCRIBE",
        "EXECUTABLE_ATTACHMENT",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} missing from {names:?}"
        );
    }
    assert!(spam::builtin(spammy).is_spam);

    assert!(rules(b"To: u@example.com\r\n\r\nbody").contains(&"MISSING_SUBJECT".to_string()));
    assert!(rules(b"Subject: x\r\n\r\nhttp://999.1.1.1/ http://1.2.3/")
        .iter()
        .all(|n| n != "NUMERIC_HTTP_ADDR"));
    // Many "://" without hosts stay fast.
    let pathological = format!(
        "Subject: x\r\n\r\n{}",
        "://".repeat(200_000) + &"a".repeat(100_000)
    );
    let started = std::time::Instant::now();
    spam::builtin(pathological.as_bytes());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn unreachable_spamd_falls_back_to_builtin() {
    let report = spam::analyze(b"Subject: x\r\n\r\nbody", "127.0.0.1", 1).await;
    assert_eq!(report.engine, "builtin");
    assert!(report.note.unwrap().contains("127.0.0.1:1"));
}
