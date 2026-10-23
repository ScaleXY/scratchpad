//! Exports emails as a single, self-contained `.html` file: the viewer UI (with
//! multi-device previews), fonts, icons, inline images and attachments are all
//! embedded, so the file works offline and can be hosted anywhere (cloud shares).

use std::sync::Arc;

use base64::Engine;
use scratchpad_core::{PublishRequest, Scratchpad, Visibility};
use serde::{Deserialize, Serialize};

use crate::assets;

const TEMPLATE: &str = include_str!("../assets/export/export.html");
const EXPORT_CSS: &str = include_str!("../assets/export/export.css");
const EXPORT_JS: &str = include_str!("../assets/export/export.js");

/// Attachments larger than this are listed but not embedded.
const MAX_ATTACHMENT: u64 = 10 * 1024 * 1024;
/// Total budget for embedded attachments per export.
const MAX_EMBEDDED: u64 = 25 * 1024 * 1024;
/// The raw source is embedded up to this size.
const MAX_RAW: usize = 2 * 1024 * 1024;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportOptions {
    /// Embed attachments (images referenced by the HTML are always embedded).
    pub attachments: bool,
    /// Include headers, envelope, Bcc, raw source, link check, spam report and view inspector.
    pub technical: bool,
    /// Device the preview opens with: fit, desktop, laptop, tablet, mobile or all.
    pub device: Option<String>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            attachments: true,
            technical: true,
            device: None,
        }
    }
}

#[derive(Serialize)]
struct Data {
    title: String,
    exported_at: i64,
    generator: String,
    options: ExportOptions,
    emails: Vec<serde_json::Value>,
}

/// Renders `ids` (in order) into one HTML document.
pub fn render(
    scratchpad: &Scratchpad,
    ids: &[String],
    title: Option<&str>,
    options: &ExportOptions,
) -> anyhow::Result<String> {
    if ids.is_empty() {
        anyhow::bail!("Select at least one email to export");
    }
    let mut budget = MAX_EMBEDDED;
    let mut emails = Vec::with_capacity(ids.len());
    for id in ids {
        emails.push(email_data(scratchpad, id, options, &mut budget)?);
    }
    let title = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .unwrap_or_else(|| {
            if emails.len() == 1 {
                emails[0]["subject"].as_str().unwrap_or("Email").to_string()
            } else {
                format!("{} emails", emails.len())
            }
        });
    let data = Data {
        title: title.clone(),
        exported_at: chrono::Utc::now().timestamp(),
        generator: format!("Scratchpad {}", scratchpad_core::VERSION),
        options: options.clone(),
        emails,
    };
    // `<` only appears inside JSON strings, where < is equivalent; this keeps
    // `</script>` in an email from ending the data block.
    let json = serde_json::to_string(&data)?.replace('<', "\\u003c");

    let fonts: String = assets::FONTS
        .iter()
        .filter(|(name, _)| *name != "inter-700.woff2")
        .map(|(name, bytes)| {
            let weight = name.trim_start_matches("inter-").trim_end_matches(".woff2");
            format!(
                "@font-face {{ font-family: \"Inter\"; font-style: normal; font-weight: {weight}; \
                 font-display: swap; src: url(data:font/woff2;base64,{}) format(\"woff2\"); }}\n",
                B64.encode(bytes)
            )
        })
        .collect();
    // app.css starts with @font-face rules pointing at the server; use the embedded ones.
    let app_css = &assets::APP_CSS[assets::APP_CSS.find(":root").unwrap_or(0)..];
    let css = format!("{app_css}\n{}\n{EXPORT_CSS}", assets::DEVICES_CSS);

    Ok(fill(
        TEMPLATE,
        &[
            ("TITLE", &escape(&title)),
            ("FAVICON", &B64.encode(assets::ICON)),
            ("FONTS", &fonts),
            ("CSS", &css),
            ("ICONS", sprite()),
            ("DEVICES_JS", assets::DEVICES_JS),
            ("EXPORT_JS", EXPORT_JS),
            ("DATA", &json),
        ],
    ))
}

/// Replaces `{{KEY}}` placeholders in one pass over the template, so placeholder-like
/// text inside a value (an email subject like "{{DATA}}") is never substituted.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out =
        String::with_capacity(template.len() + values.iter().map(|(_, v)| v.len()).sum::<usize>());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(len) = after.find("}}") else { break };
        match values.iter().find(|(key, _)| *key == &after[..len]) {
            Some((_, value)) => {
                out.push_str(&rest[..start]);
                out.push_str(value);
                rest = &after[len + 2..];
            }
            None => {
                out.push_str(&rest[..start + 2]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The icon sprite without its XML comment, ready to inline.
fn sprite() -> &'static str {
    let icons = assets::ICONS;
    &icons[icons.find("<svg").unwrap_or(0)..]
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn email_data(
    scratchpad: &Scratchpad,
    id: &str,
    options: &ExportOptions,
    budget: &mut u64,
) -> anyhow::Result<serde_json::Value> {
    let detail = scratchpad
        .email(id)?
        .ok_or_else(|| anyhow::anyhow!("Email {id} not found"))?;
    let s = &detail.summary;
    let html = detail.html.clone().unwrap_or_default();

    let mut attachments = Vec::new();
    for meta in &detail.attachments {
        // Inline images used by the HTML are needed for the preview itself.
        let referenced = meta
            .content_id
            .as_ref()
            .is_some_and(|cid| html.contains(&format!("cid:{cid}")));
        if !options.attachments && !referenced {
            continue;
        }
        let mut entry = serde_json::json!({
            "index": meta.index,
            "filename": meta.filename,
            "content_type": meta.content_type,
            "content_id": meta.content_id,
            "inline": meta.inline,
            "size": meta.size,
            "data": null,
            "omitted": null,
        });
        if meta.size > MAX_ATTACHMENT || meta.size > *budget {
            entry["omitted"] = "Too large to include in the shared file".into();
        } else if let Some((_, bytes)) = scratchpad.attachment(id, meta.index)? {
            *budget -= meta.size;
            entry["data"] = B64.encode(bytes).into();
        }
        attachments.push(entry);
    }

    let mut email = serde_json::json!({
        "id": s.id,
        "subject": s.subject,
        "from": detail.from,
        "to": detail.to,
        "cc": detail.cc,
        "bcc": [],
        "reply_to": detail.reply_to,
        "date": detail.date,
        "received_at": s.received_at,
        "size": s.size,
        "message_id": detail.message_id,
        "html": detail.html,
        "text": detail.text,
        "attachments": attachments,
        "headers": null,
        "links": null,
        "spam": null,
        "view_inspector": null,
        "raw": null,
        "raw_omitted": null,
    });
    if options.technical {
        email["bcc"] = serde_json::to_value(&detail.bcc)?;
        email["envelope_from"] = detail.envelope_from.clone().into();
        email["envelope_to"] = serde_json::to_value(&detail.envelope_to)?;
        email["headers"] = serde_json::to_value(&detail.headers)?;
        email["links"] = serde_json::to_value(&detail.links)?;
        email["spam"] = serde_json::to_value(&detail.spam)?;
        email["view_inspector"] = serde_json::to_value(&detail.view_inspector)?;
        if let Some(raw) = scratchpad.raw(id)? {
            if raw.len() <= MAX_RAW {
                email["raw"] = String::from_utf8_lossy(&raw).replace("\r\n", "\n").into();
            } else {
                email["raw_omitted"] = raw.len().into();
            }
        }
    }
    Ok(email)
}

/// A file name for an export.
pub fn file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().chars().take(80).collect::<String>();
    format!(
        "{}.html",
        if cleaned.is_empty() {
            "email"
        } else {
            &cleaned
        }
    )
}

/// What to share and where.
#[derive(Debug, Clone, Deserialize)]
pub struct CloudShareRequest {
    #[serde(default)]
    pub title: String,
    pub ids: Vec<String>,
    pub config_id: String,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    #[serde(default)]
    pub expires_in_secs: Option<u64>,
    #[serde(default)]
    pub options: ExportOptions,
}

/// Exports the emails and uploads the file to the chosen S3 config.
pub async fn share_to_cloud(
    scratchpad: &Arc<Scratchpad>,
    request: CloudShareRequest,
) -> anyhow::Result<scratchpad_core::CloudShare> {
    let title = request.title.trim().to_string();
    let html = {
        let scratchpad = scratchpad.clone();
        let ids = request.ids.clone();
        let options = request.options.clone();
        let title = title.clone();
        tokio::task::spawn_blocking(move || {
            render(
                &scratchpad,
                &ids,
                Some(&title).filter(|t| !t.is_empty()).map(|t| t.as_str()),
                &options,
            )
        })
        .await??
    };
    let title = if title.is_empty() {
        html_title(&html)
    } else {
        title
    };
    scratchpad
        .publish_cloud_share(
            PublishRequest {
                title,
                email_ids: request.ids,
                config_id: request.config_id,
                visibility: request.visibility,
                expires_in_secs: request.expires_in_secs,
            },
            html.into_bytes(),
        )
        .await
}

fn html_title(html: &str) -> String {
    html.find("<title>")
        .and_then(|start| {
            let rest = &html[start + 7..];
            rest.find("</title>").map(|end| rest[..end].to_string())
        })
        .map(|t| {
            t.replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&amp;", "&")
        })
        .unwrap_or_else(|| "Shared email".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scratchpad_core::Settings;

    const EMAIL: &str = "From: Ada <ada@example.com>\r\nTo: b@example.com\r\nBcc: secret@example.com\r\nSubject: Hello </script><b>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<html><head></head><body><img src=\"cid:logo\"><script>alert(1)</script></body></html>\r\n--x\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--x--\r\n--m\r\nContent-Type: application/pdf; name=a.pdf\r\nContent-Disposition: attachment; filename=a.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--m--\r\n";

    fn data(html: &str) -> serde_json::Value {
        let start = html
            .find("<script id=\"scratchpad-data\" type=\"application/json\">")
            .unwrap();
        let rest = &html[start..];
        let json = &rest[rest.find('>').unwrap() + 1..rest.find("</script>").unwrap()];
        serde_json::from_str(json).unwrap()
    }

    #[tokio::test]
    async fn self_contained_and_safe() {
        let scratchpad = Scratchpad::in_memory(Settings {
            link_checker: false,
            spam_check: false,
            ..Settings::default()
        })
        .unwrap();
        let id = scratchpad
            .ingest_local(EMAIL.as_bytes().to_vec(), "Default", String::new(), vec![])
            .unwrap();
        let html = render(
            &scratchpad,
            std::slice::from_ref(&id),
            None,
            &ExportOptions::default(),
        )
        .unwrap();
        // Nothing loads from the server that produced it.
        assert!(!html.contains("/assets/"));
        assert!(!html.contains("{{"));
        assert!(html.contains("<title>Hello &lt;/script&gt;&lt;b&gt;</title>"));
        // The email's markup can't break out of the data block.
        assert_eq!(html.matches("</script>").count(), 4);
        let d = data(&html);
        let email = &d["emails"][0];
        assert_eq!(email["subject"], "Hello </script><b>");
        assert_eq!(email["attachments"].as_array().unwrap().len(), 2);
        assert!(email["attachments"][0]["data"].is_string());
        assert_eq!(email["bcc"][0]["email"], "secret@example.com");

        // Without attachments/technical details: only the inline image, no Bcc or source.
        let html = render(
            &scratchpad,
            &[id],
            Some("Mine"),
            &ExportOptions {
                attachments: false,
                technical: false,
                device: Some("mobile".into()),
            },
        )
        .unwrap();
        let d = data(&html);
        let email = &d["emails"][0];
        assert_eq!(d["title"], "Mine");
        assert_eq!(d["options"]["device"], "mobile");
        assert_eq!(email["attachments"].as_array().unwrap().len(), 1);
        assert_eq!(email["attachments"][0]["content_id"], "logo");
        assert_eq!(email["bcc"].as_array().unwrap().len(), 0);
        assert!(email["raw"].is_null() && email["headers"].is_null());
        assert!(!html.contains("secret@example.com"));
    }

    #[tokio::test]
    async fn placeholders_in_titles_are_not_expanded() {
        let scratchpad = Scratchpad::in_memory(Settings {
            link_checker: false,
            spam_check: false,
            ..Settings::default()
        })
        .unwrap();
        let raw = "From: a@example.com\r\nSubject: {{DATA}} {{CSS}}\r\n\r\nhi\r\n";
        let id = scratchpad
            .ingest_local(raw.as_bytes().to_vec(), "Default", String::new(), vec![])
            .unwrap();
        let html = render(&scratchpad, &[id], None, &ExportOptions::default()).unwrap();
        assert!(html.contains("<title>{{DATA}} {{CSS}}</title>"));
        assert_eq!(data(&html)["emails"][0]["subject"], "{{DATA}} {{CSS}}");
    }

    #[test]
    fn fill_replaces_known_keys_once() {
        assert_eq!(
            fill("a{{X}}b{{Y}}c{{Z}}", &[("X", "{{Y}}"), ("Y", "2")]),
            "a{{Y}}b2c{{Z}}"
        );
        assert_eq!(fill("{{X", &[("X", "1")]), "{{X");
    }
}
