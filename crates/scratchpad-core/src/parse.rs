//! MIME parsing helpers built on `mail-parser`.

use base64::Engine;
use mail_parser::{Message, MessageParser, MimeHeaders};

use crate::model::{Address, Attachment, EmailDetail, EmailSummary, HeaderField, ViewInspector};

pub fn parse(raw: &[u8]) -> Option<Message<'_>> {
    MessageParser::default().parse(raw)
}

/// Fields extracted when an email is stored.
pub struct Overview {
    pub subject: String,
    pub from: String,
    pub to: Vec<String>,
    pub preview: String,
    pub has_attachments: bool,
}

pub fn overview(raw: &[u8]) -> Overview {
    let Some(message) = parse(raw) else {
        return Overview {
            subject: "(unparseable message)".into(),
            from: String::new(),
            to: Vec::new(),
            preview: String::new(),
            has_attachments: false,
        };
    };
    let from = addresses(message.from())
        .into_iter()
        .next()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let to = addresses(message.to())
        .into_iter()
        .map(|a| a.to_string())
        .collect();
    let preview = message
        .body_text(0)
        .map(|text| collapse_whitespace(prefix(&text, PREVIEW_SOURCE)))
        .or_else(|| {
            message.body_html(0).map(|html| {
                collapse_whitespace(&html_to_text(prefix(&html, PREVIEW_SOURCE * 8), 200))
            })
        })
        .unwrap_or_default();
    let has_attachments = message.attachments().any(|part| !is_inline(part));
    Overview {
        subject: message.subject().unwrap_or("(no subject)").to_string(),
        from,
        to,
        preview: preview.chars().take(160).collect(),
        has_attachments,
    }
}

/// Previews only need the start of the body; huge bodies aren't processed in full.
const PREVIEW_SOURCE: usize = 4096;

/// At most `max` bytes of `text`, cut at a character boundary.
pub(crate) fn prefix(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Renders HTML as plain text. Emails are untrusted input, so a panic inside the
/// HTML renderer is caught and the raw HTML is returned instead.
pub fn html_to_text(html: &str, width: usize) -> String {
    std::panic::catch_unwind(|| html2text::from_read(html.as_bytes(), width.max(20)))
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| html.to_string())
}

fn addresses(address: Option<&mail_parser::Address<'_>>) -> Vec<Address> {
    address
        .map(|address| {
            address
                .iter()
                .filter_map(|addr| {
                    Some(Address {
                        name: addr.name().map(str::to_string),
                        email: addr.address()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_inline(part: &mail_parser::MessagePart<'_>) -> bool {
    let disposition_inline = part
        .content_disposition()
        .map(|d| d.ctype().eq_ignore_ascii_case("inline"))
        .unwrap_or(false);
    part.content_id().is_some() && (disposition_inline || part.content_disposition().is_none())
}

fn content_type(part: &mail_parser::MessagePart<'_>) -> String {
    part.content_type()
        .map(|ct| match ct.subtype() {
            Some(sub) => format!("{}/{}", ct.ctype(), sub),
            None => ct.ctype().to_string(),
        })
        .unwrap_or_else(|| "application/octet-stream".into())
}

pub fn attachments(message: &Message<'_>) -> Vec<Attachment> {
    message
        .attachments()
        .enumerate()
        .map(|(index, part)| Attachment {
            index,
            filename: part
                .attachment_name()
                .map(str::to_string)
                .unwrap_or_else(|| format!("attachment-{}", index + 1)),
            content_type: content_type(part),
            content_id: part
                .content_id()
                .map(|id| id.trim_matches(['<', '>']).to_string()),
            inline: is_inline(part),
            size: part.contents().len() as u64,
        })
        .collect()
}

/// Returns `(metadata, bytes)` for the attachment at `index`.
pub fn attachment(raw: &[u8], index: usize) -> Option<(Attachment, Vec<u8>)> {
    let message = parse(raw)?;
    let meta = attachments(&message).into_iter().nth(index)?;
    let part = message.attachments().nth(index)?;
    let bytes = match part.message() {
        Some(nested) => nested.raw_message().to_vec(),
        None => part.contents().to_vec(),
    };
    Some((meta, bytes))
}

/// The HTML body with `cid:` references rewritten by `rewrite(index)`.
pub fn html_with_cids(raw: &[u8], rewrite: impl Fn(usize) -> String) -> Option<String> {
    let message = parse(raw)?;
    let html = message.body_html(0)?;
    let cids: Vec<(String, usize)> = attachments(&message)
        .into_iter()
        .filter_map(|a| Some((a.content_id?, a.index)))
        .collect();
    Some(rewrite_cids(&html, &cids, rewrite))
}

/// Replaces each whole `cid:<id>` reference (case-insensitive scheme, optionally
/// percent-encoded id) whose id is in `cids`. Unknown references are kept.
fn rewrite_cids(html: &str, cids: &[(String, usize)], rewrite: impl Fn(usize) -> String) -> String {
    if cids.is_empty() {
        return html.to_string();
    }
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut copied = 0;
    let mut search = 0;
    while let Some(found) = lower[search..].find("cid:") {
        let start = search + found;
        let id_start = start + 4;
        let id_len = html[id_start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | '<'))
            .unwrap_or(html.len() - id_start);
        let id = &html[id_start..id_start + id_len];
        let decoded = percent_decode(id);
        let target = cids
            .iter()
            .find(|(cid, _)| cid == id || cid == &decoded)
            .map(|(_, index)| *index);
        if let Some(index) = target {
            out.push_str(&html[copied..start]);
            out.push_str(&rewrite(index));
            copied = id_start + id_len;
        }
        search = id_start + id_len;
    }
    out.push_str(&html[copied..]);
    out
}

pub(crate) fn percent_decode(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn decode_b64_header(message: &Message<'_>, name: &str) -> Option<String> {
    // Long headers may arrive RFC 2047 encoded (e.g. when folded by Python's email package).
    let raw = decode_encoded_words(message.header_raw(name)?);
    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact)
        .ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (!text.is_empty()).then_some(text)
}

/// Decodes RFC 2047 encoded words (`=?charset?Q|B?text?=`); other text is kept as is.
fn decode_encoded_words(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("=?") {
        let (before, word) = rest.split_at(start);
        let parsed = word[2..].splitn(3, '?').collect::<Vec<_>>();
        let decoded = (parsed.len() == 3).then(|| {
            let end = parsed[2].find("?=")?;
            let text = &parsed[2][..end];
            let bytes = match parsed[1] {
                "B" | "b" => base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .ok()?,
                "Q" | "q" => decode_q(text),
                _ => return None,
            };
            let consumed = 2 + parsed[0].len() + 1 + parsed[1].len() + 1 + end + 2;
            Some((String::from_utf8_lossy(&bytes).into_owned(), consumed))
        });
        match decoded.flatten() {
            Some((text, consumed)) => {
                // Whitespace between adjacent encoded words is not significant.
                if !before.trim().is_empty() || out.is_empty() {
                    out.push_str(before);
                }
                out.push_str(&text);
                rest = &word[consumed..];
            }
            None => {
                out.push_str(&rest[..start + 2]);
                rest = &rest[start + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_q(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'_' => out.push(b' '),
            b'=' => {
                let hex = bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok());
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'='),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    out
}

pub fn view_inspector(message: &Message<'_>) -> Option<ViewInspector> {
    let inspector = ViewInspector {
        view_file: decode_b64_header(message, "X-HELO-View-File"),
        view_source: decode_b64_header(message, "X-HELO-View"),
        view_data_html: decode_b64_header(message, "X-HELO-View-Data"),
    };
    (inspector.view_file.is_some()
        || inspector.view_source.is_some()
        || inspector.view_data_html.is_some())
    .then_some(inspector)
}

pub fn detail(
    raw: &[u8],
    summary: EmailSummary,
    envelope_from: String,
    envelope_to: Vec<String>,
) -> EmailDetail {
    let Some(message) = parse(raw) else {
        return EmailDetail {
            summary,
            from: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            reply_to: vec![],
            envelope_from,
            envelope_to,
            date: None,
            message_id: None,
            html: None,
            text: Some(String::from_utf8_lossy(raw).into_owned()),
            headers: vec![],
            attachments: vec![],
            view_inspector: None,
            links: None,
            spam: None,
        };
    };

    let to = addresses(message.to());
    let cc = addresses(message.cc());
    let mut bcc = addresses(message.bcc());
    // SMTP envelope recipients missing from To/Cc were Bcc'd.
    for rcpt in &envelope_to {
        let listed = to
            .iter()
            .chain(cc.iter())
            .chain(bcc.iter())
            .any(|a| a.email.eq_ignore_ascii_case(rcpt));
        if !listed {
            bcc.push(Address {
                name: None,
                email: rcpt.clone(),
            });
        }
    }

    let text = message
        .text_bodies()
        .next()
        .filter(|part| !part.is_text_html())
        .and_then(|part| part.text_contents())
        .map(str::to_string);
    EmailDetail {
        text,
        from: addresses(message.from()),
        to,
        cc,
        bcc,
        reply_to: addresses(message.reply_to()),
        envelope_from,
        envelope_to,
        date: message.date().map(|d| d.to_rfc3339()),
        message_id: message.message_id().map(str::to_string),
        html: message.body_html(0).map(|h| h.into_owned()),
        headers: message
            .headers_raw()
            .map(|(name, value)| HeaderField {
                name: name.to_string(),
                value: unfold(value),
            })
            .collect(),
        attachments: attachments(&message),
        view_inspector: view_inspector(&message),
        summary,
        links: None,
        spam: None,
    }
}

fn unfold(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "From: App <app@example.com>\r\nTo: user@example.com\r\nSubject: Welcome\r\nX-HELO-View-File: ZW1haWxzLndlbGNvbWU=\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nHello there\r\n--b\r\nContent-Type: text/html\r\n\r\n<p>Hello <a href=\"https://example.com\">there</a></p>\r\n--b--\r\n";

    #[test]
    fn overview_and_detail() {
        let o = overview(SAMPLE.as_bytes());
        assert_eq!(o.subject, "Welcome");
        assert_eq!(o.from, "App <app@example.com>");
        assert_eq!(o.preview, "Hello there");

        let summary = EmailSummary {
            id: "1".into(),
            mailbox: "Default".into(),
            subject: o.subject,
            from: o.from,
            to: o.to,
            preview: o.preview,
            received_at: 0,
            size: 0,
            read: false,
            has_attachments: false,
            spam_score: None,
            broken_links: None,
        };
        let d = detail(
            SAMPLE.as_bytes(),
            summary,
            "app@example.com".into(),
            vec!["user@example.com".into(), "hidden@example.com".into()],
        );
        assert_eq!(d.bcc.len(), 1);
        assert_eq!(d.text.as_deref().map(str::trim), Some("Hello there"));
        assert!(d.html.unwrap().contains("<a href"));
        assert_eq!(
            d.view_inspector.unwrap().view_file.as_deref(),
            Some("emails.welcome")
        );
    }

    #[test]
    fn decodes_rfc2047_encoded_inspector_headers() {
        let raw = "Subject: x\r\nX-HELO-View: =?utf-8?q?QGNvbXBvbmVudCgnbWFpbDo6bWVzc2FnZScpCiMgSW52b2ljZSB7e?=\r\n =?utf-8?q?yAkaW52b2ljZS0+aWQgfX0KVGhhbmtzIQpAZW5kY29tcG9uZW50?=\r\n\r\nbody\r\n";
        let message = parse(raw.as_bytes()).unwrap();
        let inspector = view_inspector(&message).unwrap();
        assert!(inspector
            .view_source
            .unwrap()
            .starts_with("@component('mail::message')"));
    }

    #[test]
    fn decodes_encoded_words() {
        assert_eq!(
            decode_encoded_words("=?utf-8?q?a=3Db_c?= =?UTF-8?B?ZA==?= plain"),
            "a=b cd plain"
        );
        assert_eq!(decode_encoded_words("no words"), "no words");
    }
}
