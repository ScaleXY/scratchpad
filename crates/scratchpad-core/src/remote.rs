//! Remote mailboxes: periodically pull mail from IMAP accounts or other Scratchpad instances.

use std::sync::Arc;

use futures::StreamExt;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::model::{EmailDetail, EmailSummary, RemoteMailbox, RemoteSource};
use crate::Scratchpad;

/// Stores a pulled message off the async runtime (parsing and SQLite are blocking).
async fn ingest(
    scratchpad: &Arc<Scratchpad>,
    raw: Vec<u8>,
    mailbox: &str,
    from: String,
    to: Vec<String>,
    external_id: String,
) -> anyhow::Result<bool> {
    let scratchpad = scratchpad.clone();
    let mailbox = mailbox.to_string();
    let id = tokio::task::spawn_blocking(move || {
        scratchpad.ingest_remote(raw, &mailbox, from, to, external_id)
    })
    .await??;
    Ok(id.is_some())
}

/// Polls one remote mailbox and imports new messages. Returns the number imported.
pub async fn poll(scratchpad: &Arc<Scratchpad>, remote: &RemoteMailbox) -> anyhow::Result<usize> {
    match &remote.source {
        RemoteSource::Imap {
            host,
            port,
            tls,
            username,
            password,
            folder,
            delete_after_fetch,
        } => {
            let tcp = tokio::time::timeout(
                std::time::Duration::from_secs(15),
                TcpStream::connect((host.as_str(), *port)),
            )
            .await??;
            if *tls {
                let connector = tokio_rustls::TlsConnector::from(crate::tls::client_config());
                let name = ServerName::try_from(host.clone())?;
                let stream = connector.connect(name, tcp).await?;
                poll_imap(
                    scratchpad,
                    remote,
                    stream,
                    username,
                    password,
                    folder,
                    *delete_after_fetch,
                )
                .await
            } else {
                poll_imap(
                    scratchpad,
                    remote,
                    tcp,
                    username,
                    password,
                    folder,
                    *delete_after_fetch,
                )
                .await
            }
        }
        RemoteSource::Scratchpad {
            base_url,
            username,
            password,
            mailbox,
        } => {
            poll_scratchpad(
                scratchpad,
                remote,
                base_url,
                username.as_deref(),
                password.as_deref(),
                mailbox,
            )
            .await
        }
    }
}

async fn poll_imap<T>(
    scratchpad: &Arc<Scratchpad>,
    remote: &RemoteMailbox,
    stream: T,
    username: &str,
    password: &str,
    folder: &str,
    delete_after_fetch: bool,
) -> anyhow::Result<usize>
where
    T: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let mut client = async_imap::Client::new(stream);
    let _greeting = client.read_response().await?;
    let mut session = client
        .login(username, password)
        .await
        .map_err(|(error, _)| error)?;
    let selected = session.select(folder).await?;
    let validity = selected.uid_validity.unwrap_or_default();

    let uids = session.uid_search("ALL").await?;
    let mut new_uids: Vec<u32> = uids
        .into_iter()
        .filter(|uid| {
            !scratchpad
                .store
                .has_external_id(&remote.name, &format!("imap:{validity}:{uid}"))
                .unwrap_or(false)
        })
        .collect();
    new_uids.sort_unstable();

    let mut imported = 0;
    for chunk in new_uids.chunks(25) {
        let set = chunk
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let fetched: Vec<(u32, Vec<u8>)> = {
            let mut stream = session.uid_fetch(&set, "(UID BODY.PEEK[])").await?;
            let mut out = Vec::new();
            while let Some(fetch) = stream.next().await {
                let fetch = fetch?;
                if let (Some(uid), Some(body)) = (fetch.uid, fetch.body()) {
                    out.push((uid, body.to_vec()));
                }
            }
            out
        };
        let mut stored = Vec::new();
        for (uid, raw) in fetched {
            let external_id = format!("imap:{validity}:{uid}");
            if ingest(
                scratchpad,
                raw,
                &remote.name,
                String::new(),
                vec![],
                external_id,
            )
            .await?
            {
                imported += 1;
            }
            stored.push(uid.to_string());
        }
        // Only messages that were actually stored are deleted from the server.
        if delete_after_fetch && !stored.is_empty() {
            let mut stream = session
                .uid_store(stored.join(","), "+FLAGS.SILENT (\\Deleted)")
                .await?;
            while stream.next().await.is_some() {}
        }
    }
    if delete_after_fetch && !new_uids.is_empty() {
        let stream = session.expunge().await?;
        futures::pin_mut!(stream);
        while stream.next().await.is_some() {}
    }
    let _ = session.logout().await;
    Ok(imported)
}

async fn poll_scratchpad(
    scratchpad: &Arc<Scratchpad>,
    remote: &RemoteMailbox,
    base_url: &str,
    username: Option<&str>,
    password: Option<&str>,
    mailbox: &str,
) -> anyhow::Result<usize> {
    // Credentials embedded in the URL would end up in error messages (shown in the UI
    // and logged), so they are sent as basic auth instead.
    let (base, url_user, url_password) = split_credentials(base_url.trim());
    let base = base.trim_end_matches('/');
    let (username, password) = match username.filter(|u| !u.is_empty()) {
        Some(user) => (Some(user.to_string()), password.map(str::to_string)),
        None => (url_user, url_password),
    };
    let auth = |request: reqwest::RequestBuilder| match &username {
        Some(user) => request.basic_auth(user, password.as_deref()),
        None => request,
    };
    let list_url = format!("{base}/api/mailboxes/{}/emails", urlencode(mailbox));
    let summaries: Vec<EmailSummary> = auth(scratchpad.http.get(&list_url))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let mut imported = 0;
    for summary in summaries.into_iter().rev() {
        let external_id = format!("scratchpad:{}", summary.id);
        if scratchpad
            .store
            .has_external_id(&remote.name, &external_id)?
        {
            continue;
        }
        let id = urlencode(&summary.id);
        let detail: EmailDetail = auth(scratchpad.http.get(format!("{base}/api/emails/{id}")))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let raw = auth(scratchpad.http.get(format!("{base}/api/emails/{id}/raw")))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        if ingest(
            scratchpad,
            raw.to_vec(),
            &remote.name,
            detail.envelope_from,
            detail.envelope_to,
            external_id,
        )
        .await?
        {
            imported += 1;
        }
    }
    Ok(imported)
}

/// Splits `https://user:pass@host/…` into the URL without credentials and the credentials.
fn split_credentials(base_url: &str) -> (String, Option<String>, Option<String>) {
    match url::Url::parse(base_url) {
        Ok(mut url) if !url.username().is_empty() || url.password().is_some() => {
            let user = Some(crate::parse::percent_decode(url.username()));
            let password = url.password().map(crate::parse::percent_decode);
            let _ = url.set_username("");
            let _ = url.set_password(None);
            (url.to_string(), user, password)
        }
        _ => (base_url.to_string(), None, None),
    }
}

fn urlencode(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_moved_out_of_the_url() {
        let (url, user, password) =
            split_credentials("https://admin:p%40ss@scratchpad.example.com/x");
        assert_eq!(url, "https://scratchpad.example.com/x");
        assert_eq!(user.as_deref(), Some("admin"));
        assert_eq!(password.as_deref(), Some("p@ss"));
        let (url, user, _) = split_credentials("http://127.0.0.1:8025");
        assert_eq!((url.as_str(), user), ("http://127.0.0.1:8025", None));
    }

    #[test]
    fn encodes_path_segments() {
        assert_eq!(urlencode("My Box/../x"), "My%20Box%2F..%2Fx");
    }
}
