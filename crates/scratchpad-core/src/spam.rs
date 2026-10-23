//! Spam report: SpamAssassin through the spamd protocol, with built-in heuristics as a fallback.

use std::time::Duration;

use mail_parser::MimeHeaders;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::model::{SpamReport, SpamRule};

const THRESHOLD: f64 = 5.0;
/// Connecting, sending the message and getting the report together.
const SPAMD_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_SPAMD_RESPONSE: u64 = 1024 * 1024;

pub async fn analyze(raw: &[u8], spamd_host: &str, spamd_port: u16) -> SpamReport {
    let result = match tokio::time::timeout(
        SPAMD_TIMEOUT,
        spamd_report(raw, spamd_host, spamd_port),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("timed out")),
    };
    match result {
        Ok(report) => report,
        Err(error) => {
            // The heuristics parse the whole message: keep that off the async runtime.
            let owned = raw.to_vec();
            let mut report = tokio::task::spawn_blocking(move || builtin(&owned))
                .await
                .unwrap_or_else(|_| builtin(b""));
            report.note = Some(format!(
                "SpamAssassin (spamd) is not reachable at {spamd_host}:{spamd_port} ({error}). \
                 Showing Scratchpad's built-in heuristics instead. Install SpamAssassin and run `spamd` for full reports."
            ));
            report
        }
    }
}

async fn spamd_report(raw: &[u8], host: &str, port: u16) -> anyhow::Result<SpamReport> {
    let mut stream =
        tokio::time::timeout(Duration::from_secs(3), TcpStream::connect((host, port))).await??;
    let header = format!(
        "REPORT SPAMC/1.5\r\nContent-length: {}\r\nUser: scratchpad\r\n\r\n",
        raw.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(raw).await?;
    stream.shutdown().await?;
    let mut response = Vec::new();
    (&mut stream)
        .take(MAX_SPAMD_RESPONSE)
        .read_to_end(&mut response)
        .await?;
    parse_spamd_response(&String::from_utf8_lossy(&response))
}

fn parse_spamd_response(response: &str) -> anyhow::Result<SpamReport> {
    let mut lines = response.split("\r\n");
    let status = lines.next().unwrap_or_default();
    if !status.starts_with("SPAMD/") || !status.contains("EX_OK") {
        anyhow::bail!("unexpected spamd response: {status}");
    }
    let mut score = 0.0;
    let mut threshold = THRESHOLD;
    let mut is_spam = false;
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Spam:") {
            // "True ; 15.0 / 5.0"
            let (flag, numbers) = value.split_once(';').unwrap_or((value, ""));
            is_spam =
                flag.trim().eq_ignore_ascii_case("true") || flag.trim().eq_ignore_ascii_case("yes");
            let mut parts = numbers.split('/').map(|n| n.trim().parse::<f64>());
            if let Some(Ok(s)) = parts.next() {
                score = s;
            }
            if let Some(Ok(t)) = parts.next() {
                threshold = t;
            }
        }
    }
    let body: Vec<&str> = lines.collect();
    let body = body.join("\n");
    Ok(SpamReport {
        score,
        threshold,
        is_spam,
        rules: parse_report_table(&body),
        engine: "spamassassin".into(),
        note: None,
    })
}

fn parse_report_table(report: &str) -> Vec<SpamRule> {
    let mut rules: Vec<SpamRule> = Vec::new();
    let mut in_table = false;
    for line in report.lines() {
        if line.starts_with("----") {
            in_table = true;
            continue;
        }
        if !in_table || line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let first = parts.next().unwrap_or_default();
        match first.parse::<f64>() {
            Ok(score) => {
                let name = parts.next().unwrap_or_default().to_string();
                let description = parts.collect::<Vec<_>>().join(" ");
                rules.push(SpamRule {
                    name,
                    score,
                    description,
                });
            }
            Err(_) => {
                if let Some(last) = rules.last_mut() {
                    last.description.push(' ');
                    last.description.push_str(line.trim());
                }
            }
        }
    }
    rules
}

/// A small, transparent rule set covering common deliverability problems.
pub fn builtin(raw: &[u8]) -> SpamReport {
    let mut rules = Vec::new();
    let mut add = |name: &str, score: f64, description: &str| {
        rules.push(SpamRule {
            name: name.into(),
            score,
            description: description.into(),
        });
    };

    match crate::parse::parse(raw) {
        None => add("UNPARSEABLE", 5.0, "Message could not be parsed as MIME"),
        Some(message) => {
            let subject = message.subject().unwrap_or_default();
            if subject.trim().is_empty() {
                add("MISSING_SUBJECT", 1.8, "Missing Subject: header");
            } else {
                let letters: Vec<char> = subject.chars().filter(|c| c.is_alphabetic()).collect();
                if letters.len() >= 8 && letters.iter().all(|c| c.is_uppercase()) {
                    add("SUBJ_ALL_CAPS", 0.5, "Subject is all capitals");
                }
                if subject.matches('!').count() >= 3 {
                    add(
                        "SUBJ_EXCESS_EXCLAIM",
                        0.6,
                        "Subject contains many exclamation marks",
                    );
                }
            }
            if message.date().is_none() {
                add("MISSING_DATE", 1.4, "Missing Date: header");
            }
            if message.message_id().is_none() {
                add("MISSING_MID", 0.5, "Missing Message-Id: header");
            }
            if message.from().is_none() {
                add("MISSING_FROM", 1.0, "Missing From: header");
            } else if let Some(addr) = message.from().and_then(|f| f.first()) {
                if let (Some(name), Some(email)) = (addr.name(), addr.address()) {
                    if name.contains('@') && !name.eq_ignore_ascii_case(email) {
                        add(
                            "FROM_NAME_ADDR_MISMATCH",
                            1.0,
                            "From: display name contains a different email address",
                        );
                    }
                }
            }
            if message.to().is_none() {
                add("MISSING_HEADERS", 1.0, "Missing To: header");
            }
            // mail-parser also lists a plain text part among the HTML bodies, so look
            // for a real text/html part.
            if message.mime_version().is_empty()
                && (message.html_bodies().any(|p| p.is_text_html())
                    || message.attachment_count() > 0)
            {
                add(
                    "MISSING_MIME_VERSION",
                    0.5,
                    "MIME message without MIME-Version header",
                );
            }

            let html = message.body_html(0).map(|h| h.into_owned());
            let has_text_part = message.text_bodies().any(|p| !p.is_text_html());
            let html_lower = html.as_deref().map(str::to_ascii_lowercase);
            if let (Some(html), Some(lower)) = (&html, &html_lower) {
                if !has_text_part {
                    add(
                        "MIME_HTML_ONLY",
                        0.7,
                        "Message only has text/html MIME parts",
                    );
                }
                let text = crate::parse::html_to_text(crate::parse::prefix(html, 256 * 1024), 80);
                let text_len = text.split_whitespace().count();
                let images = lower.matches("<img").count();
                if images > 0 && text_len < 30 {
                    add(
                        "HTML_IMAGE_RATIO",
                        1.0,
                        "HTML has a low ratio of text to images",
                    );
                }
                if lower.contains("<script") {
                    add(
                        "HTML_SCRIPT",
                        2.0,
                        "HTML contains <script> (stripped by most clients)",
                    );
                }
                if lower.contains("<form") {
                    add("HTML_FORM", 0.8, "HTML contains a form");
                }
            }

            let lower = html_lower
                .or_else(|| message.body_text(0).map(|t| t.to_ascii_lowercase()))
                .unwrap_or_default();
            for (needle, name) in [
                ("bit.ly/", "URL_SHORTENER"),
                ("tinyurl.com/", "URL_SHORTENER"),
                ("goo.gl/", "URL_SHORTENER"),
            ] {
                if lower.contains(needle) {
                    add(name, 1.2, "Body links through a URL shortener");
                    break;
                }
            }
            if has_ip_url(&lower) {
                add(
                    "NUMERIC_HTTP_ADDR",
                    1.5,
                    "URI contains a numeric IP address",
                );
            }
            let spam_phrases = [
                "100% free",
                "act now",
                "click here",
                "winner",
                "risk-free",
                "no credit check",
                "earn money",
                "limited time offer",
            ];
            let hits: Vec<&str> = spam_phrases
                .iter()
                .copied()
                .filter(|p| lower.contains(p))
                .collect();
            if !hits.is_empty() {
                add(
                    "SPAMMY_PHRASES",
                    0.4 * hits.len() as f64,
                    &format!("Body contains common spam phrases: {}", hits.join(", ")),
                );
            }
            if message.list_unsubscribe().is_empty() {
                add(
                    "NO_LIST_UNSUBSCRIBE",
                    0.1,
                    "No List-Unsubscribe header (required by Gmail/Yahoo for bulk mail)",
                );
            }
            let large_attachment = message
                .attachments()
                .any(|a| a.contents().len() > 10 * 1024 * 1024);
            if large_attachment {
                add("LARGE_ATTACHMENT", 0.5, "Attachment larger than 10 MB");
            }
            if message.attachments().any(|a| {
                a.attachment_name()
                    .map(|n| {
                        let n = n.to_ascii_lowercase();
                        [".exe", ".scr", ".bat", ".js", ".vbs", ".cmd"]
                            .iter()
                            .any(|ext| n.ends_with(ext))
                    })
                    .unwrap_or(false)
            }) {
                add("EXECUTABLE_ATTACHMENT", 3.0, "Executable attachment");
            }
        }
    }

    let score = (rules.iter().map(|r| r.score).sum::<f64>() * 10.0).round() / 10.0;
    SpamReport {
        score,
        threshold: THRESHOLD,
        is_spam: score >= THRESHOLD,
        rules,
        engine: "builtin".into(),
        note: None,
    }
}

fn has_ip_url(lower: &str) -> bool {
    lower.match_indices("://").any(|(index, _)| {
        // Bounded, so bodies full of "://" stay linear.
        let host: String = lower[index + 3..]
            .chars()
            .take(64)
            .take_while(|c| {
                !c.is_whitespace() && !matches!(c, '/' | ':' | '"' | '\'' | '<' | '>' | '?' | '#')
            })
            .collect();
        let parts: Vec<&str> = host.split('.').collect();
        parts.len() == 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.parse::<u8>().is_ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spamd_report() {
        let response = "SPAMD/1.1 0 EX_OK\r\nContent-length: 10\r\nSpam: False ; 1.9 / 5.0\r\n\r\nContent analysis details:   (1.9 points, 5.0 required)\n\n pts rule name              description\n---- ---------------------- --------------------------------------------------\n 1.4 MISSING_DATE           Missing Date: header\n 0.5 MISSING_MID            Missing Message-Id:\n                            header\n";
        let report = parse_spamd_response(response).unwrap();
        assert_eq!(report.score, 1.9);
        assert!(!report.is_spam);
        assert_eq!(report.rules.len(), 2);
        assert_eq!(report.rules[1].description, "Missing Message-Id: header");
    }

    #[test]
    fn builtin_flags_missing_headers() {
        let report = builtin(b"Subject: HELLO WORLD NOW\r\n\r\nclick here http://10.0.0.1/x");
        let names: Vec<_> = report.rules.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"MISSING_DATE"));
        assert!(names.contains(&"SUBJ_ALL_CAPS"));
        assert!(names.contains(&"NUMERIC_HTTP_ADDR"));
    }
}
