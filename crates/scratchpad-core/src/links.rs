//! Link checker: HEAD every link, falling back to GET when HEAD fails.

use std::collections::HashSet;
use std::time::Duration;

use futures::stream::{self, StreamExt};

use crate::model::{LinkCheck, LinkStatus};

/// At most this many links are extracted (and checked) per email.
pub const MAX_LINKS: usize = 500;
/// Link labels are looked for this far after the `href`.
const MAX_LABEL_SCAN: usize = 4096;
const MAX_LABEL_CHARS: usize = 200;

/// Extracts unique links (`href` attributes from HTML, bare URLs from text) in document
/// order, at most [`MAX_LINKS`].
pub fn extract(html: Option<&str>, text: Option<&str>) -> Vec<(String, Option<String>)> {
    let mut links: Vec<(String, Option<String>)> = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |url: String, label: Option<String>| {
        let url = decode_entities(url.trim());
        if url.is_empty() || links.len() >= MAX_LINKS || !seen.insert(url.clone()) {
            return;
        }
        links.push((url, label.filter(|l| !l.is_empty())));
    };

    if let Some(html) = html {
        let lower = html.to_ascii_lowercase();
        let mut pos = 0;
        while let Some(found) = lower[pos..].find("href") {
            let start = pos + found + 4;
            pos = start;
            let rest = &html[start..];
            let trimmed = rest.trim_start();
            let Some(after_eq) = trimmed.strip_prefix('=') else {
                continue;
            };
            let after_eq = after_eq.trim_start();
            let (url, consumed) = match after_eq.chars().next() {
                Some(q @ ('"' | '\'')) => match after_eq[1..].find(q) {
                    Some(end) => (&after_eq[1..1 + end], end + 2),
                    None => continue,
                },
                Some(_) => {
                    let end = after_eq
                        .find(|c: char| c.is_whitespace() || c == '>')
                        .unwrap_or(after_eq.len());
                    (&after_eq[..end], end)
                }
                None => continue,
            };
            let offset = start + (rest.len() - after_eq.len()) + consumed;
            // Bounded search window, so huge documents with many links stay linear.
            let window = floor_boundary(html, (offset + MAX_LABEL_SCAN).min(html.len()));
            let label = lower[offset..window].find('>').and_then(|gt| {
                let from = offset + gt + 1;
                lower[from..window]
                    .find("</a")
                    .map(|end| strip_tags(&html[from..from + end]))
            });
            push(url.to_string(), label);
        }
    }

    if let Some(text) = text {
        for word in text.split(|c: char| {
            c.is_whitespace() || matches!(c, '<' | '>' | '"' | '(' | ')' | '[' | ']')
        }) {
            if word.starts_with("http://") || word.starts_with("https://") {
                push(
                    word.trim_end_matches(['.', ',', ';', ':', '!', '?'])
                        .to_string(),
                    None,
                );
            }
        }
    }
    links
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn strip_tags(fragment: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out.split_whitespace().collect::<Vec<_>>().join(" "))
        .chars()
        .take(MAX_LABEL_CHARS)
        .collect()
}

fn decode_entities(s: &str) -> String {
    // `&amp;` last, so `&amp;lt;` becomes `&lt;` rather than `<`.
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

pub fn pending(links: &[(String, Option<String>)]) -> Vec<LinkCheck> {
    links
        .iter()
        .map(|(url, text)| LinkCheck {
            url: url.clone(),
            text: text.clone(),
            status: initial_status(url),
        })
        .collect()
}

fn initial_status(url: &str) -> LinkStatus {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        LinkStatus::Pending
    } else if lower.starts_with("mailto:") || lower.starts_with("tel:") || lower.starts_with("sms:")
    {
        LinkStatus::Skipped {
            reason: "not an HTTP link".into(),
        }
    } else if lower.starts_with('#') {
        LinkStatus::Skipped {
            reason: "anchor".into(),
        }
    } else if lower.starts_with("{{") || lower.contains("%7b%7b") || lower.contains("{{") {
        LinkStatus::Broken {
            status: None,
            error: "unrendered template placeholder".into(),
        }
    } else {
        LinkStatus::Broken {
            status: None,
            error: "relative or unsupported URL".into(),
        }
    }
}

async fn check_one(client: &reqwest::Client, url: &str, timeout: Duration) -> LinkStatus {
    let head = client.head(url).timeout(timeout).send().await;
    match head {
        Ok(response) if response.status().is_success() => {
            return LinkStatus::Ok {
                status: response.status().as_u16(),
                method: "HEAD".into(),
            }
        }
        _ => {}
    }
    match client.get(url).timeout(timeout).send().await {
        Ok(response) if response.status().is_success() => LinkStatus::Ok {
            status: response.status().as_u16(),
            method: "GET".into(),
        },
        Ok(response) => LinkStatus::Broken {
            status: Some(response.status().as_u16()),
            error: response
                .status()
                .canonical_reason()
                .unwrap_or("HTTP error")
                .to_string(),
        },
        Err(error) => LinkStatus::Broken {
            status: None,
            error: describe(&error),
        },
    }
}

/// A short, human readable reason (the root cause) for a failed request.
fn describe(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return "timed out".into();
    }
    let mut root: &dyn std::error::Error = error;
    while let Some(source) = root.source() {
        root = source;
    }
    let reason = root.to_string();
    if error.is_connect() {
        format!("connection failed: {reason}")
    } else {
        reason
    }
}

pub async fn check_all(
    client: &reqwest::Client,
    mut links: Vec<LinkCheck>,
    timeout: Duration,
) -> Vec<LinkCheck> {
    let todo: Vec<(usize, String)> = links
        .iter()
        .enumerate()
        .filter(|(_, link)| link.status == LinkStatus::Pending)
        .map(|(index, link)| (index, link.url.clone()))
        .collect();
    let results: Vec<(usize, LinkStatus)> = stream::iter(todo)
        .map(|(index, url)| async move { (index, check_one(client, &url, timeout).await) })
        .buffer_unordered(8)
        .collect()
        .await;
    for (index, status) in results {
        links[index].status = status;
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_links() {
        let html = r#"<a href="https://a.test/x?a=1&amp;b=2">Go <b>now</b></a> <a href='mailto:x@y'>m</a><a href=https://b.test>b</a>"#;
        let links = extract(
            Some(html),
            Some("see https://c.test/page. and https://a.test/x?a=1&b=2"),
        );
        let urls: Vec<_> = links.iter().map(|(u, _)| u.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://a.test/x?a=1&b=2",
                "mailto:x@y",
                "https://b.test",
                "https://c.test/page"
            ]
        );
        assert_eq!(links[0].1.as_deref(), Some("Go now"));
    }
}
