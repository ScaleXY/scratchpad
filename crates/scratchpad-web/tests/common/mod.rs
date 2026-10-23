//! Shared helpers for the integration tests: an in-memory Scratchpad with its web server.
#![allow(dead_code)]

use std::sync::Arc;

use scratchpad_core::{Scratchpad, Settings};

/// Starts the web server for an in-memory instance (checks off) on a random port.
pub async fn serve(settings: Settings) -> (Arc<Scratchpad>, String) {
    let scratchpad = Scratchpad::in_memory(Settings {
        link_checker: false,
        spam_check: false,
        ..settings
    })
    .unwrap();
    let server = scratchpad_web::WebServer::bind(scratchpad.clone(), "127.0.0.1", 0)
        .await
        .unwrap();
    let url = server.url();
    scratchpad.set_web_url(Some(url.clone()));
    tokio::spawn(server.serve());
    (scratchpad, url)
}

pub fn ingest(scratchpad: &Arc<Scratchpad>, raw: &str, mailbox: &str) -> String {
    scratchpad
        .ingest_local(
            raw.as_bytes().to_vec(),
            mailbox,
            "a@example.com".into(),
            vec!["b@example.com".into()],
        )
        .unwrap()
}

pub fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default()
}
