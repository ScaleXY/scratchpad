//! SMTP protocol edge cases against the real server (`Scratchpad::start` on a free port).

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use scratchpad_core::{Scratchpad, Settings};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn start(customize: impl FnOnce(&mut Settings)) -> Arc<Scratchpad> {
    let mut settings = Settings {
        smtp_port: free_port(),
        link_checker: false,
        spam_check: false,
        ..Settings::default()
    };
    customize(&mut settings);
    let scratchpad = Scratchpad::in_memory(settings).unwrap();
    scratchpad.start();
    for _ in 0..100 {
        if scratchpad.status().smtp_running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        scratchpad.status().smtp_running,
        "{:?}",
        scratchpad.status().smtp_error
    );
    scratchpad
}

fn b64(input: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(input)
}

struct Client<S> {
    stream: BufReader<S>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Client<S> {
    /// Reads one (possibly multi-line) reply and returns all its lines.
    async fn reply_lines(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            let read =
                tokio::time::timeout(Duration::from_secs(10), self.stream.read_line(&mut line))
                    .await
                    .expect("reply in time")
                    .unwrap();
            assert!(read > 0, "connection closed, replies so far: {lines:?}");
            let line = line.trim_end().to_string();
            let last = line.len() < 4 || line.as_bytes()[3] != b'-';
            lines.push(line);
            if last {
                return lines;
            }
        }
    }

    async fn reply(&mut self) -> String {
        self.reply_lines().await.pop().unwrap()
    }

    async fn send(&mut self, data: &[u8]) {
        self.stream.get_mut().write_all(data).await.unwrap();
    }

    async fn cmd(&mut self, line: &str) -> String {
        self.send(format!("{line}\r\n").as_bytes()).await;
        self.reply().await
    }

    async fn expect(&mut self, line: &str, code: &str) {
        let reply = self.cmd(line).await;
        assert!(
            reply.starts_with(code),
            "{line} -> {reply} (expected {code})"
        );
    }

    async fn closed(&mut self) -> bool {
        let mut rest = Vec::new();
        matches!(
            tokio::time::timeout(
                Duration::from_secs(5),
                self.stream.read_until(b'\n', &mut rest)
            )
            .await,
            Ok(Ok(0)) | Ok(Err(_))
        )
    }
}

async fn connect(scratchpad: &Scratchpad) -> Client<TcpStream> {
    let stream = TcpStream::connect(scratchpad.status().smtp_address)
        .await
        .unwrap();
    let mut client = Client {
        stream: BufReader::new(stream),
    };
    assert!(client.reply().await.starts_with("220"));
    client
}

async fn deliver<S: AsyncRead + AsyncWrite + Unpin>(client: &mut Client<S>, subject: &str) {
    client.expect("MAIL FROM:<a@example.com>", "250").await;
    client.expect("RCPT TO:<b@example.com>", "250").await;
    client.expect("DATA", "354").await;
    client
        .expect(&format!("Subject: {subject}\r\n\r\nbody\r\n."), "250")
        .await;
}

#[tokio::test]
async fn auth_plain_variants() {
    let scratchpad = start(|_| {}).await;
    let mut c = connect(&scratchpad).await;
    c.expect("EHLO test", "250").await;

    // Unknown mechanism, undecodable and incomplete credentials.
    c.expect("AUTH CRAM-MD5", "504").await;
    c.expect("AUTH PLAIN !!!", "501").await;
    c.expect(&format!("AUTH PLAIN {}", b64("no-nul-separators")), "501")
        .await;
    // Cancelling the exchange.
    c.expect("AUTH PLAIN", "334").await;
    c.expect("*", "501").await;

    // Initial response.
    c.expect(&format!("AUTH PLAIN {}", b64("\0Inbox One\0secret")), "235")
        .await;
    c.expect(&format!("AUTH PLAIN {}", b64("\0x\0y")), "503")
        .await;
    deliver(&mut c, "one").await;
    c.expect("QUIT", "221").await;

    // Continuation; authzid only (some clients send `user\0\0password`).
    let mut c = connect(&scratchpad).await;
    c.expect("EHLO test", "250").await;
    c.expect("AUTH PLAIN", "334").await;
    c.expect(&b64("Inbox Two\0\0secret"), "235").await;
    deliver(&mut c, "two").await;

    assert_eq!(scratchpad.emails(Some("Inbox One"), None).unwrap().len(), 1);
    assert_eq!(scratchpad.emails(Some("Inbox Two"), None).unwrap().len(), 1);
}

#[tokio::test]
async fn auth_login_variants() {
    let scratchpad = start(|_| {}).await;
    let mut c = connect(&scratchpad).await;
    c.expect("EHLO test", "250").await;
    // Initial response with the username.
    c.expect(&format!("AUTH LOGIN {}", b64("Login Box")), "334")
        .await;
    c.expect(&b64("password"), "235").await;
    deliver(&mut c, "login").await;
    assert_eq!(scratchpad.emails(Some("Login Box"), None).unwrap().len(), 1);

    let mut c = connect(&scratchpad).await;
    c.expect("AUTH LOGIN", "334").await;
    c.expect("*", "501").await;
    c.expect("AUTH LOGIN", "334").await;
    c.expect(&b64("user"), "334").await;
    c.expect("*", "501").await;
    // Not authenticated after cancelling: mail goes to Default.
    deliver(&mut c, "default").await;
    assert_eq!(scratchpad.emails(Some("Default"), None).unwrap().len(), 1);

    // A closed connection during AUTH doesn't authenticate or crash anything.
    let mut c = connect(&scratchpad).await;
    c.expect("AUTH LOGIN", "334").await;
    drop(c);
    let mut c = connect(&scratchpad).await;
    c.expect("NOOP", "250").await;
}

#[tokio::test]
async fn sequencing_and_misc_commands() {
    let scratchpad = start(|_| {}).await;
    let mut c = connect(&scratchpad).await;
    c.expect("RCPT TO:<b@example.com>", "503").await;
    c.expect("DATA", "503").await;
    c.expect("MAIL FROM:<a@example.com>", "250").await;
    c.expect("DATA", "503").await;
    c.expect("RCPT TO:<>", "501").await;
    c.expect("RCPT b@example.com", "501").await;
    c.expect("MAIL TO:<a@example.com>", "501").await;
    c.expect("RSET", "250").await;
    c.expect("RCPT TO:<b@example.com>", "503").await;
    c.expect("VRFY someone", "252").await;
    c.expect("NOOP", "250").await;
    c.expect("HELP", "214").await;
    c.expect("BDAT 10 LAST", "502").await;
    c.expect("STARTTLS", "502").await;
    c.expect("FOO bar", "502").await;
    c.expect("", "500").await;
    c.expect("helo lowercase", "250").await;

    let ehlo = {
        c.send(b"EHLO test\r\n").await;
        c.reply_lines().await
    };
    assert!(ehlo.iter().any(|l| l.contains("SIZE 52428800")), "{ehlo:?}");
    assert!(ehlo.iter().any(|l| l.contains("PIPELINING")));
    assert!(!ehlo.iter().any(|l| l.contains("STARTTLS")));

    // An empty reverse path (bounces) is allowed; mail still lands.
    c.expect("MAIL FROM:<>", "250").await;
    c.expect("RCPT TO:<b@example.com>", "250").await;
    c.expect("DATA", "354").await;
    c.expect("Subject: bounce\r\n\r\nx\r\n.", "250").await;
    c.expect("QUIT", "221").await;
    assert!(c.closed().await);
    assert_eq!(scratchpad.emails(None, None).unwrap().len(), 1);
}

#[tokio::test]
async fn oversized_lines_are_rejected_and_skipped() {
    let scratchpad = start(|_| {}).await;
    let mut c = connect(&scratchpad).await;
    let mut long = b"NOOP ".to_vec();
    long.extend(std::iter::repeat_n(b'x', 200 * 1024));
    long.extend_from_slice(b"\r\n");
    c.send(&long).await;
    assert!(c.reply().await.starts_with("500"));
    // The rest of the long line was not interpreted as further commands.
    c.expect("NOOP", "250").await;
}

#[tokio::test]
async fn size_limit_is_enforced() {
    let scratchpad = start(|s| s.max_message_size = 1024).await;
    let mut c = connect(&scratchpad).await;
    c.expect("EHLO test", "250").await;
    // Declared size over the limit.
    c.expect("MAIL FROM:<a@example.com> SIZE=4096", "552").await;
    c.expect("MAIL FROM:<a@example.com> SIZE=100", "250").await;
    c.expect("RCPT TO:<b@example.com>", "250").await;
    c.expect("DATA", "354").await;
    let mut body = b"Subject: big\r\n\r\n".to_vec();
    for _ in 0..100 {
        body.extend_from_slice(b"0123456789012345678901234567890123456789\r\n");
    }
    body.extend_from_slice(b".\r\n");
    c.send(&body).await;
    assert!(c.reply().await.starts_with("552"));
    // The transaction was reset, the session goes on.
    c.expect("RCPT TO:<b@example.com>", "503").await;
    deliver(&mut c, "small").await;
    let emails = scratchpad.emails(None, None).unwrap();
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].subject, "small");
}

#[tokio::test]
async fn zero_size_limit_means_default() {
    let scratchpad = start(|s| s.max_message_size = 0).await;
    let mut c = connect(&scratchpad).await;
    c.send(b"EHLO test\r\n").await;
    let ehlo = c.reply_lines().await;
    assert!(ehlo.iter().any(|l| l.contains("SIZE 52428800")), "{ehlo:?}");
    deliver(&mut c, "fine").await;
}

#[tokio::test]
async fn pipelined_transactions() {
    let scratchpad = start(|_| {}).await;
    let mut c = connect(&scratchpad).await;
    let mut batch = String::from("EHLO pipelined\r\n");
    for i in 0..3 {
        batch.push_str(&format!(
            "MAIL FROM:<a@example.com>\r\nRCPT TO:<b{i}@example.com>\r\nRCPT TO:<c{i}@example.com>\r\nDATA\r\n"
        ));
        batch.push_str(&format!("Subject: pipelined {i}\r\n\r\n.. dotted\r\n.\r\n"));
    }
    batch.push_str("QUIT\r\n");
    c.send(batch.as_bytes()).await;
    c.reply_lines().await; // EHLO
    for _ in 0..3 {
        for code in ["250", "250", "250", "354", "250"] {
            let reply = c.reply().await;
            assert!(reply.starts_with(code), "expected {code}, got {reply}");
        }
    }
    assert!(c.reply().await.starts_with("221"));
    let emails = scratchpad.emails(None, None).unwrap();
    assert_eq!(emails.len(), 3);
    let detail = scratchpad.email(&emails[0].id).unwrap().unwrap();
    assert_eq!(detail.envelope_to.len(), 2);
    let raw = scratchpad.raw(&emails[0].id).unwrap().unwrap();
    assert!(raw.ends_with(b"\r\n. dotted\r\n"));
}

#[tokio::test]
async fn bare_lf_clients_work() {
    let scratchpad = start(|_| {}).await;
    let mut c = connect(&scratchpad).await;
    c.send(b"HELO lf\nMAIL FROM:<a@b.c>\nRCPT TO:<d@e.f>\nDATA\n")
        .await;
    for code in ["250", "250", "250", "354"] {
        assert!(c.reply().await.starts_with(code));
    }
    c.send(b"Subject: lf\n\nbody\n.\n").await;
    assert!(c.reply().await.starts_with("250"));
    assert_eq!(scratchpad.emails(None, None).unwrap()[0].subject, "lf");
}

#[tokio::test]
async fn required_auth_rejects_anonymous_mail() {
    let scratchpad = start(|s| s.smtp_require_auth = true).await;
    let mut c = connect(&scratchpad).await;
    c.expect("EHLO test", "250").await;
    c.expect("MAIL FROM:<a@example.com>", "530").await;
    c.expect(&format!("AUTH PLAIN {}", b64("\0Secure\0pw")), "235")
        .await;
    deliver(&mut c, "authed").await;
    assert_eq!(scratchpad.emails(Some("Secure"), None).unwrap().len(), 1);
}

#[tokio::test]
async fn restarting_on_a_new_port() {
    let scratchpad = start(|_| {}).await;
    let old = scratchpad.status().smtp_address;
    let port = free_port();
    scratchpad
        .update_settings(Settings {
            smtp_port: port,
            ..scratchpad.settings()
        })
        .unwrap();
    for _ in 0..100 {
        if scratchpad
            .status()
            .smtp_address
            .ends_with(&format!(":{port}"))
            && scratchpad.status().smtp_running
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_ne!(scratchpad.status().smtp_address, old);
    let mut c = connect(&scratchpad).await;
    c.expect("NOOP", "250").await;
    // Starting twice doesn't start a second server that fights over the port.
    scratchpad.start();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(scratchpad.status().smtp_running);
}

mod no_verify {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};

    /// Accepts any certificate: the server uses a self-signed one.
    #[derive(Debug)]
    pub struct NoVerify;

    impl ServerCertVerifier for NoVerify {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }
}

#[tokio::test]
async fn starttls_upgrade() {
    let scratchpad = start(|s| s.smtp_starttls = true).await;
    let mut c = connect(&scratchpad).await;
    c.send(b"EHLO plain\r\n").await;
    assert!(c.reply_lines().await.iter().any(|l| l.contains("STARTTLS")));
    // Authentication before STARTTLS is forgotten after the upgrade.
    c.expect(&format!("AUTH PLAIN {}", b64("\0Before\0pw")), "235")
        .await;
    c.expect("STARTTLS extra", "501").await;
    c.expect("STARTTLS", "220").await;

    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(no_verify::NoVerify))
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let tls = connector
        .connect(
            rustls::pki_types::ServerName::try_from("localhost").unwrap(),
            c.stream.into_inner(),
        )
        .await
        .unwrap();
    let mut c = Client {
        stream: BufReader::new(tls),
    };
    c.send(b"EHLO secure\r\n").await;
    let ehlo = c.reply_lines().await;
    assert!(!ehlo.iter().any(|l| l.contains("STARTTLS")), "{ehlo:?}");
    c.expect("STARTTLS", "502").await;
    deliver(&mut c, "encrypted").await;
    c.expect("QUIT", "221").await;
    let emails = scratchpad.emails(Some("Default"), None).unwrap();
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].subject, "encrypted");
}

#[tokio::test]
async fn plaintext_pipelined_after_starttls_is_not_injected() {
    let scratchpad = start(|s| s.smtp_starttls = true).await;
    let mut c = connect(&scratchpad).await;
    c.expect("EHLO plain", "250").await;
    // Commands sent in clear text right after STARTTLS must not run inside TLS.
    c.send(b"STARTTLS\r\nMAIL FROM:<injected@example.com>\r\n")
        .await;
    assert!(c.reply().await.starts_with("220"));
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(no_verify::NoVerify))
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let server_name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    // The injected bytes are either dropped (handshake succeeds, no MAIL state) or
    // break the handshake; both are safe.
    if let Ok(tls) = connector.connect(server_name, c.stream.into_inner()).await {
        let mut c = Client {
            stream: BufReader::new(tls),
        };
        c.expect("EHLO secure", "250").await;
        c.expect("RCPT TO:<b@example.com>", "503").await;
    }
}

#[tokio::test]
async fn binary_and_8bit_payloads_are_kept_verbatim() {
    let scratchpad = start(|_| {}).await;
    let mut c = connect(&scratchpad).await;
    c.expect("EHLO test", "250").await;
    c.expect("MAIL FROM:<a@example.com> BODY=8BITMIME SMTPUTF8", "250")
        .await;
    c.expect("RCPT TO:<ü@example.com>", "250").await;
    c.expect("DATA", "354").await;
    let mut body = "Subject: Grüße\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nÄÖÜ "
        .as_bytes()
        .to_vec();
    body.extend_from_slice(&[0xff, 0xfe, 0x00, b'\r', b'\n', b'.', b'\r', b'\n']);
    c.send(&body).await;
    assert!(c.reply().await.starts_with("250"));
    let email = &scratchpad.emails(None, None).unwrap()[0];
    assert_eq!(email.subject, "Grüße");
    let raw = scratchpad.raw(&email.id).unwrap().unwrap();
    assert!(raw.ends_with(&[0xff, 0xfe, 0x00, b'\r', b'\n']));
}
