//! A permissive SMTP server that captures every message it receives.
//!
//! Any credentials are accepted, the SMTP username selects the mailbox, and
//! unauthenticated deliveries land in the "Default" mailbox.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;

use crate::model::DEFAULT_MAILBOX;
use crate::Scratchpad;

/// Longest command line accepted (RFC 5321 only requires 512 octets).
const MAX_LINE: usize = 64 * 1024;
/// DATA is read in chunks of at most this many bytes, so long lines don't need one big buffer.
const MAX_DATA_CHUNK: usize = 1024 * 1024;
const MAX_RECIPIENTS: usize = 1000;
/// Mailbox names come from the SMTP username; longer names are truncated.
const MAX_MAILBOX_NAME: usize = 200;
/// Simultaneous SMTP sessions; further connections are turned away with a 421.
const MAX_CONNECTIONS: usize = 256;
/// Clients sending this many failing commands are disconnected.
const MAX_ERRORS: usize = 25;
/// Idle time allowed for each read or write (RFC 5321 suggests 5 minutes).
const IO_TIMEOUT: Duration = Duration::from_secs(300);

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub(crate) async fn accept_loop(
    listener: TcpListener,
    scratchpad: Arc<Scratchpad>,
    tls: Option<TlsAcceptor>,
) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (mut stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::warn!(%error, "SMTP accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            tracing::warn!(%peer, "too many SMTP connections, rejecting");
            tokio::spawn(async move {
                let busy = b"421 4.7.0 Too many connections, try again later\r\n";
                let _ = tokio::time::timeout(Duration::from_secs(5), stream.write_all(busy)).await;
            });
            continue;
        };
        let scratchpad = scratchpad.clone();
        let tls = tls.clone();
        tokio::spawn(async move {
            let _permit = permit;
            tracing::debug!(%peer, "SMTP connection");
            if let Err(error) = Session::new(Box::new(stream), scratchpad, tls).run().await {
                tracing::debug!(%peer, %error, "SMTP session ended with error");
            }
        });
    }
}

struct Session {
    io: Option<BufReader<Box<dyn Io>>>,
    scratchpad: Arc<Scratchpad>,
    tls: Option<TlsAcceptor>,
    timeout: Duration,
    secure: bool,
    greeted: bool,
    errors: usize,
    username: Option<String>,
    mail_from: Option<String>,
    rcpt_to: Vec<String>,
}

enum Flow {
    Continue,
    Quit,
}

/// One line read from the client.
enum Input {
    Line(String),
    /// The line exceeded [`MAX_LINE`]; it was discarded.
    TooLong,
}

impl Session {
    fn new(stream: Box<dyn Io>, scratchpad: Arc<Scratchpad>, tls: Option<TlsAcceptor>) -> Self {
        Self {
            io: Some(BufReader::new(stream)),
            scratchpad,
            tls,
            timeout: IO_TIMEOUT,
            secure: false,
            greeted: false,
            errors: 0,
            username: None,
            mail_from: None,
            rcpt_to: Vec::new(),
        }
    }

    fn io(&mut self) -> &mut BufReader<Box<dyn Io>> {
        self.io.as_mut().expect("stream present")
    }

    async fn reply(&mut self, line: &str) -> anyhow::Result<()> {
        if line.starts_with('5') {
            self.errors += 1;
        }
        let timeout = self.timeout;
        let io = self.io().get_mut();
        tokio::time::timeout(timeout, async {
            io.write_all(line.as_bytes()).await?;
            io.write_all(b"\r\n").await?;
            io.flush().await
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out writing to client"))??;
        Ok(())
    }

    /// Reads up to `limit` bytes ending in `\n` (or less at EOF). Errors when the client is idle too long.
    async fn read_chunk(&mut self, limit: usize, buf: &mut Vec<u8>) -> anyhow::Result<usize> {
        let timeout = self.timeout;
        let mut reader = AsyncReadExt::take(self.io(), limit as u64);
        let read = reader.read_until(b'\n', buf);
        match tokio::time::timeout(timeout, read).await {
            Ok(read) => Ok(read?),
            Err(_) => {
                let _ = self.reply("421 4.4.2 Timeout, closing connection").await;
                anyhow::bail!("client timed out")
            }
        }
    }

    /// Reads a command line. `None` when the client closed the connection.
    async fn read_line(&mut self) -> anyhow::Result<Option<Input>> {
        let mut buf = Vec::new();
        if self.read_chunk(MAX_LINE, &mut buf).await? == 0 {
            return Ok(None);
        }
        if !buf.ends_with(b"\n") && buf.len() >= MAX_LINE {
            // Skip the rest of the oversized line so it isn't read as commands.
            loop {
                buf.clear();
                if self.read_chunk(MAX_LINE, &mut buf).await? == 0 {
                    return Ok(None);
                }
                if buf.ends_with(b"\n") {
                    return Ok(Some(Input::TooLong));
                }
            }
        }
        while matches!(buf.last(), Some(b'\n' | b'\r')) {
            buf.pop();
        }
        Ok(Some(Input::Line(
            String::from_utf8_lossy(&buf).into_owned(),
        )))
    }

    async fn run(mut self) -> anyhow::Result<()> {
        self.reply("220 scratchpad.local ESMTP Scratchpad ready")
            .await?;
        while let Some(input) = self.read_line().await? {
            let flow = match input {
                Input::Line(line) => self.command(&line).await?,
                Input::TooLong => {
                    self.reply("500 5.5.6 Line too long").await?;
                    Flow::Continue
                }
            };
            if matches!(flow, Flow::Quit) {
                break;
            }
            if self.errors >= MAX_ERRORS {
                self.reply("421 4.7.0 Too many errors, closing connection")
                    .await?;
                break;
            }
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.mail_from = None;
        self.rcpt_to.clear();
    }

    fn max_size(&self) -> u64 {
        self.scratchpad.settings().message_size_limit()
    }

    async fn command(&mut self, line: &str) -> anyhow::Result<Flow> {
        let (verb, args) = match line.split_once([' ', '\t']) {
            Some((verb, args)) => (verb.to_ascii_uppercase(), args.trim()),
            None => (line.trim().to_ascii_uppercase(), ""),
        };
        match verb.as_str() {
            "EHLO" => {
                self.greeted = true;
                self.reset();
                let max = self.max_size();
                let mut lines = vec![
                    "250-scratchpad.local greets you".to_string(),
                    "250-PIPELINING".into(),
                    format!("250-SIZE {max}"),
                    "250-8BITMIME".into(),
                    "250-SMTPUTF8".into(),
                    "250-AUTH PLAIN LOGIN".into(),
                    "250-AUTH=PLAIN LOGIN".into(),
                ];
                if self.tls.is_some() && !self.secure {
                    lines.push("250-STARTTLS".into());
                }
                lines.push("250 ENHANCEDSTATUSCODES".into());
                self.reply(&lines.join("\r\n")).await?;
            }
            "HELO" => {
                self.greeted = true;
                self.reset();
                self.reply("250 scratchpad.local").await?;
            }
            "STARTTLS" => {
                if !args.is_empty() {
                    self.reply("501 5.5.4 Syntax: STARTTLS").await?;
                    return Ok(Flow::Continue);
                }
                let Some(acceptor) = self.tls.clone().filter(|_| !self.secure) else {
                    self.reply("502 5.5.1 STARTTLS not available").await?;
                    return Ok(Flow::Continue);
                };
                self.reply("220 2.0.0 Ready to start TLS").await?;
                // `into_inner` drops anything pipelined after STARTTLS in plain text, so it
                // can't be injected into the encrypted session (CVE-2011-0411).
                let stream = self.io.take().expect("stream present").into_inner();
                let tls_stream = tokio::time::timeout(self.timeout, acceptor.accept(stream))
                    .await
                    .map_err(|_| anyhow::anyhow!("TLS handshake timed out"))??;
                self.io = Some(BufReader::new(Box::new(tls_stream)));
                self.secure = true;
                self.greeted = false;
                self.username = None;
                self.reset();
            }
            "AUTH" => self.auth(args).await?,
            "MAIL" => {
                if self.scratchpad.settings().smtp_require_auth && self.username.is_none() {
                    self.reply("530 5.7.0 Authentication required").await?;
                    return Ok(Flow::Continue);
                }
                let max = self.max_size();
                match parse_path(args, "FROM:") {
                    Some(_) if declared_size(args).is_some_and(|size| size > max) => {
                        self.reply("552 5.3.4 Message size exceeds limit").await?
                    }
                    Some(from) => {
                        self.reset();
                        self.mail_from = Some(from);
                        self.reply("250 2.1.0 Ok").await?;
                    }
                    None => self.reply("501 5.5.4 Syntax: MAIL FROM:<address>").await?,
                }
            }
            "RCPT" => {
                if self.mail_from.is_none() {
                    self.reply("503 5.5.1 Need MAIL command first").await?;
                    return Ok(Flow::Continue);
                }
                match parse_path(args, "TO:") {
                    Some(_) if self.rcpt_to.len() >= MAX_RECIPIENTS => {
                        self.reply("452 4.5.3 Too many recipients").await?
                    }
                    Some(to) if !to.is_empty() => {
                        self.rcpt_to.push(to);
                        self.reply("250 2.1.5 Ok").await?;
                    }
                    _ => self.reply("501 5.5.4 Syntax: RCPT TO:<address>").await?,
                }
            }
            "DATA" => {
                if self.mail_from.is_none() || self.rcpt_to.is_empty() {
                    self.reply("503 5.5.1 Need RCPT command first").await?;
                    return Ok(Flow::Continue);
                }
                self.reply("354 End data with <CR><LF>.<CR><LF>").await?;
                match self.read_data().await? {
                    Some(raw) => {
                        let mailbox = self
                            .username
                            .clone()
                            .filter(|name| !name.is_empty())
                            .unwrap_or_else(|| DEFAULT_MAILBOX.to_string());
                        let from = self.mail_from.take().unwrap_or_default();
                        let to = std::mem::take(&mut self.rcpt_to);
                        // Parsing and storing a large message is CPU and disk bound.
                        let scratchpad = self.scratchpad.clone();
                        let stored = tokio::task::spawn_blocking(move || {
                            scratchpad.ingest_local(raw, &mailbox, from, to)
                        })
                        .await
                        .map_err(anyhow::Error::from)
                        .and_then(|result| result);
                        match stored {
                            Ok(id) => self.reply(&format!("250 2.0.0 Ok: queued as {id}")).await?,
                            Err(error) => {
                                tracing::error!(error = %format!("{error:#}"), "failed to store email");
                                self.reply("451 4.3.0 Failed to store message").await?
                            }
                        }
                    }
                    None => {
                        self.reset();
                        self.reply("552 5.3.4 Message size exceeds limit").await?;
                    }
                }
            }
            "BDAT" => {
                self.reply("502 5.5.1 CHUNKING is not supported, use DATA")
                    .await?
            }
            "RSET" => {
                self.reset();
                self.reply("250 2.0.0 Ok").await?;
            }
            "NOOP" => self.reply("250 2.0.0 Ok").await?,
            "VRFY" => {
                self.reply("252 2.1.5 Cannot VRFY user, but will accept message")
                    .await?
            }
            "HELP" => self.reply("214 2.0.0 Scratchpad captures all mail").await?,
            "QUIT" => {
                self.reply("221 2.0.0 Bye").await?;
                return Ok(Flow::Quit);
            }
            "" => self.reply("500 5.5.2 Empty command").await?,
            _ => self.reply("502 5.5.2 Command not implemented").await?,
        }
        Ok(Flow::Continue)
    }

    /// Reads one SASL response line. `None` when the client cancelled with `*`.
    async fn sasl_response(&mut self, challenge: &str) -> anyhow::Result<Option<String>> {
        self.reply(challenge).await?;
        match self.read_line().await? {
            Some(Input::Line(line)) if line.trim() == "*" => Ok(None),
            Some(Input::Line(line)) => Ok(Some(line)),
            Some(Input::TooLong) => Ok(Some(String::new())),
            None => anyhow::bail!("connection closed during AUTH"),
        }
    }

    async fn auth(&mut self, args: &str) -> anyhow::Result<()> {
        if self.username.is_some() {
            self.reply("503 5.5.1 Already authenticated").await?;
            return Ok(());
        }
        let (mechanism, initial) = match args.split_once(' ') {
            Some((m, rest)) => (m.to_ascii_uppercase(), Some(rest.trim().to_string())),
            None => (args.to_ascii_uppercase(), None),
        };
        let cancelled = "501 5.7.0 Authentication cancelled";
        let username = match mechanism.as_str() {
            "PLAIN" => {
                let response = match initial.filter(|s| !s.is_empty() && s != "=") {
                    Some(response) => response,
                    None => match self.sasl_response("334 ").await? {
                        Some(response) => response,
                        None => return self.reply(cancelled).await,
                    },
                };
                if response.trim() == "*" {
                    return self.reply(cancelled).await;
                }
                decode_b64(&response).and_then(|decoded| {
                    // authzid NUL authcid NUL password
                    let mut parts = decoded.split('\0');
                    let authzid = parts.next()?;
                    let authcid = parts.next()?;
                    parts.next()?;
                    Some(if authcid.is_empty() { authzid } else { authcid }.to_string())
                })
            }
            "LOGIN" => {
                let user = match initial.filter(|s| !s.is_empty() && s != "=") {
                    Some(user) => user,
                    None => match self.sasl_response("334 VXNlcm5hbWU6").await? {
                        Some(user) => user,
                        None => return self.reply(cancelled).await,
                    },
                };
                // The password is read and discarded: any password is accepted.
                if self.sasl_response("334 UGFzc3dvcmQ6").await?.is_none() {
                    return self.reply(cancelled).await;
                }
                decode_b64(&user)
            }
            _ => {
                self.reply("504 5.5.4 Unrecognized authentication type")
                    .await?;
                return Ok(());
            }
        };
        match username {
            Some(username) => {
                self.username = Some(mailbox_name(&username));
                self.reply("235 2.7.0 Authentication successful").await?;
            }
            None => self.reply("501 5.5.2 Cannot decode credentials").await?,
        }
        Ok(())
    }

    /// Reads the DATA payload. Returns `None` when the message exceeds the size limit.
    async fn read_data(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        let max = usize::try_from(self.max_size()).unwrap_or(usize::MAX);
        let mut data = Vec::new();
        let mut too_large = false;
        // Lines longer than one chunk arrive in pieces; only a piece that starts a
        // line can be the terminator or carry a stuffed dot.
        let mut line_start = true;
        let mut line = Vec::new();
        loop {
            line.clear();
            if self.read_chunk(MAX_DATA_CHUNK, &mut line).await? == 0 {
                anyhow::bail!("connection closed during DATA");
            }
            if line_start && (line == b".\r\n" || line == b".\n") {
                break;
            }
            let content = if line_start && line.starts_with(b"..") {
                &line[1..]
            } else {
                &line[..]
            };
            line_start = line.ends_with(b"\n");
            if too_large {
                continue;
            }
            if data.len() + content.len() > max {
                // Keep reading until the terminator, but stop buffering.
                too_large = true;
                data = Vec::new();
            } else {
                data.extend_from_slice(content);
            }
        }
        Ok((!too_large).then_some(data))
    }
}

fn decode_b64(input: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input.trim())
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// A mailbox name from an SMTP username: trimmed, without control characters, length-capped.
fn mailbox_name(username: &str) -> String {
    username
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_MAILBOX_NAME)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The `SIZE=` parameter of `MAIL FROM` (RFC 1870).
fn declared_size(args: &str) -> Option<u64> {
    args.split_whitespace().find_map(|param| {
        let (key, value) = param.split_once('=')?;
        key.eq_ignore_ascii_case("SIZE")
            .then(|| value.parse().ok())
            .flatten()
    })
}

/// Parses `FROM:<addr> [params]` / `TO:<addr> [params]`.
fn parse_path(args: &str, prefix: &str) -> Option<String> {
    let head = args.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = args[prefix.len()..].trim_start();
    let address = if let Some(stripped) = rest.strip_prefix('<') {
        &stripped[..stripped.find('>')?]
    } else {
        rest.split_whitespace().next().unwrap_or_default()
    };
    Some(address.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Settings;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

    #[test]
    fn parses_paths() {
        assert_eq!(
            parse_path("FROM:<a@b.c> SIZE=12", "FROM:").as_deref(),
            Some("a@b.c")
        );
        assert_eq!(parse_path("from: <>", "FROM:").as_deref(), Some(""));
        assert_eq!(parse_path("TO:x@y.z", "TO:").as_deref(), Some("x@y.z"));
        assert_eq!(parse_path("TO <x@y.z>", "FROM:"), None);
        assert_eq!(parse_path("TO:<unterminated", "TO:"), None);
        // Multi-byte input must not panic when slicing the prefix.
        assert_eq!(parse_path("é", "FROM:"), None);
        assert_eq!(parse_path("FRÖM:<a@b>", "FROM:"), None);
    }

    #[test]
    fn parses_declared_size() {
        assert_eq!(
            declared_size("FROM:<a@b> SIZE=1000 BODY=8BITMIME"),
            Some(1000)
        );
        assert_eq!(declared_size("FROM:<a@b> size=7"), Some(7));
        assert_eq!(declared_size("FROM:<a@b> SIZE=abc"), None);
        assert_eq!(declared_size("FROM:<a@b>"), None);
    }

    #[test]
    fn sanitizes_mailbox_names() {
        assert_eq!(mailbox_name("  Team\r\n "), "Team");
        assert_eq!(mailbox_name("a\0b"), "ab");
        assert_eq!(mailbox_name(&"x".repeat(1000)).len(), MAX_MAILBOX_NAME);
    }

    async fn session(timeout: Duration) -> (Arc<Scratchpad>, BufReader<DuplexStream>) {
        let scratchpad = Scratchpad::in_memory(Settings {
            link_checker: false,
            spam_check: false,
            ..Settings::default()
        })
        .unwrap();
        let (client, server) = tokio::io::duplex(1 << 16);
        let mut session = Session::new(Box::new(server), scratchpad.clone(), None);
        session.timeout = timeout;
        tokio::spawn(session.run());
        (scratchpad, BufReader::new(client))
    }

    async fn reply(client: &mut BufReader<DuplexStream>) -> String {
        let mut line = String::new();
        loop {
            line.clear();
            client.read_line(&mut line).await.unwrap();
            if line.len() < 4 || line.as_bytes()[3] != b'-' {
                return line;
            }
        }
    }

    #[tokio::test]
    async fn idle_clients_are_disconnected() {
        let (_scratchpad, mut client) = session(Duration::from_millis(100)).await;
        assert!(reply(&mut client).await.starts_with("220"));
        assert!(reply(&mut client).await.starts_with("421"));
        let mut rest = String::new();
        assert_eq!(client.read_line(&mut rest).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn too_many_errors_disconnect() {
        let (_scratchpad, mut client) = session(IO_TIMEOUT).await;
        assert!(reply(&mut client).await.starts_with("220"));
        for _ in 0..MAX_ERRORS {
            client.get_mut().write_all(b"BOGUS\r\n").await.unwrap();
            assert!(reply(&mut client).await.starts_with("502"));
        }
        assert!(reply(&mut client).await.starts_with("421"));
    }

    #[tokio::test]
    async fn dot_at_chunk_boundary_is_not_a_terminator() {
        let (scratchpad, mut client) = session(IO_TIMEOUT).await;
        assert!(reply(&mut client).await.starts_with("220"));
        for command in ["EHLO x", "MAIL FROM:<a@b.c>", "RCPT TO:<c@d.e>", "DATA"] {
            client
                .get_mut()
                .write_all(format!("{command}\r\n").as_bytes())
                .await
                .unwrap();
            reply(&mut client).await;
        }
        // A line that is split exactly before ".\r\n" by the chunked reader.
        let mut body = b"Subject: long\r\n\r\n".to_vec();
        body.extend(std::iter::repeat_n(b'a', MAX_DATA_CHUNK));
        body.extend_from_slice(b".\r\nafter\r\n.\r\n");
        client.get_mut().write_all(&body).await.unwrap();
        assert!(reply(&mut client).await.starts_with("250"));
        let emails = scratchpad.emails(None, None).unwrap();
        let raw = scratchpad.raw(&emails[0].id).unwrap().unwrap();
        assert!(raw.ends_with(b"a.\r\nafter\r\n"));
    }
}
