//! S3-compatible object storage for cloud shares (AWS S3, Cloudflare R2, MinIO,
//! Backblaze B2, Wasabi, …). Requests are signed with SigV4 query parameters
//! (presigned URLs), so the same code works against every provider.

use std::time::Duration;

use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use serde::{Deserialize, Serialize};
use url::Url;

/// Public shares live under this prefix: `<host>/email/shares/<uuid>.html`.
pub const PUBLIC_PREFIX: &str = "email/shares/";
/// Private shares get their own prefix, so a bucket policy (or CDN rule) can make
/// `email/shares/` public without exposing them.
pub const PRIVATE_PREFIX: &str = "email/private/";
/// SigV4 presigned URLs are valid for at most 7 days.
pub const MAX_PRESIGN_SECS: u64 = 7 * 24 * 3600;

/// What kind of links a bucket is set up for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum S3Mode {
    /// Objects under `email/shares/` are publicly readable (bucket policy or CDN).
    #[default]
    Public,
    /// Nothing is public; shares are presigned, expiring links.
    Private,
    /// Public links and presigned links, chosen per share.
    Both,
}

impl S3Mode {
    pub fn allows(self, visibility: Visibility) -> bool {
        matches!(
            (self, visibility),
            (S3Mode::Both, _)
                | (S3Mode::Public, Visibility::Public)
                | (S3Mode::Private, Visibility::Private)
        )
    }

    pub fn default_visibility(self) -> Visibility {
        match self {
            S3Mode::Private => Visibility::Private,
            S3Mode::Public | S3Mode::Both => Visibility::Public,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Public,
    /// A presigned link that expires.
    Private,
}

/// Where the credentials of an S3 config are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStorage {
    /// Only on this device, in the data directory.
    #[default]
    Local,
    /// In the vault: encrypted with the master password and synced to the cloud backend.
    Synced,
}

fn default_region() -> String {
    "auto".into()
}

fn yes() -> bool {
    true
}

/// A bucket that shares are uploaded to.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct S3Config {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// e.g. `https://s3.eu-west-1.amazonaws.com`, `https://<account>.r2.cloudflarestorage.com`.
    pub endpoint: String,
    #[serde(default = "default_region")]
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    #[serde(default)]
    pub secret_key: String,
    /// Optional vanity/CDN domain (CloudFront, Cloudflare, …) for public links.
    #[serde(default)]
    pub cdn_domain: String,
    #[serde(default)]
    pub mode: S3Mode,
    /// `https://endpoint/bucket/key` instead of `https://bucket.endpoint/key`.
    #[serde(default = "yes")]
    pub path_style: bool,
    #[serde(default)]
    pub storage: CredentialStorage,
}

impl std::fmt::Debug for S3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("mode", &self.mode)
            .field("storage", &self.storage)
            .finish_non_exhaustive()
    }
}

/// An S3 config without its secret, for UIs and the API.
#[derive(Debug, Clone, Serialize)]
pub struct S3ConfigInfo {
    #[serde(flatten)]
    pub config: S3Config,
    pub has_secret: bool,
}

impl S3Config {
    pub fn redacted(&self) -> S3ConfigInfo {
        S3ConfigInfo {
            has_secret: !self.secret_key.is_empty(),
            config: S3Config {
                secret_key: String::new(),
                ..self.clone()
            },
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("Give the S3 config a name");
        }
        self.endpoint_url()?;
        if self.bucket.trim().is_empty() {
            anyhow::bail!("Bucket is required");
        }
        if self.access_key.trim().is_empty() || self.secret_key.is_empty() {
            anyhow::bail!("Access key and secret key are required");
        }
        if !self.cdn_domain.trim().is_empty() {
            Url::parse(&self.cdn_base())
                .map_err(|e| anyhow::anyhow!("Invalid vanity/CDN domain: {e}"))?;
        }
        Ok(())
    }

    fn endpoint_url(&self) -> anyhow::Result<Url> {
        let endpoint = self.endpoint.trim();
        let endpoint = if endpoint.contains("://") {
            endpoint.to_string()
        } else {
            format!("https://{endpoint}")
        };
        let mut url =
            Url::parse(&endpoint).map_err(|e| anyhow::anyhow!("Invalid S3 endpoint: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            anyhow::bail!("The S3 endpoint must be an http(s) URL");
        }
        // `Bucket` joins the bucket name onto the endpoint path.
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }
        Ok(url)
    }

    fn bucket_handle(&self) -> anyhow::Result<Bucket> {
        let style = if self.path_style {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };
        let region = if self.region.trim().is_empty() {
            default_region()
        } else {
            self.region.trim().to_string()
        };
        Bucket::new(
            self.endpoint_url()?,
            style,
            self.bucket.trim().to_string(),
            region,
        )
        .map_err(|e| anyhow::anyhow!("Invalid S3 endpoint or bucket: {e:?}"))
    }

    fn credentials(&self) -> Credentials {
        Credentials::new(self.access_key.trim(), self.secret_key.clone())
    }

    fn cdn_base(&self) -> String {
        let cdn = self.cdn_domain.trim().trim_end_matches('/');
        if cdn.contains("://") {
            cdn.to_string()
        } else {
            format!("https://{cdn}")
        }
    }

    /// The object key for a share.
    pub fn object_key(visibility: Visibility, id: &str) -> String {
        let prefix = match visibility {
            Visibility::Public => PUBLIC_PREFIX,
            Visibility::Private => PRIVATE_PREFIX,
        };
        format!("{prefix}{id}.html")
    }

    /// The unsigned URL of an object: through the CDN domain when one is set.
    pub fn public_url(&self, key: &str) -> anyhow::Result<String> {
        if !self.cdn_domain.trim().is_empty() {
            return Ok(format!("{}/{key}", self.cdn_base()));
        }
        Ok(self.direct_url(key)?.to_string())
    }

    /// The object's URL on the S3 endpoint itself.
    pub fn direct_url(&self, key: &str) -> anyhow::Result<Url> {
        Ok(self.bucket_handle()?.object_url(key)?)
    }

    /// A presigned GET link, valid for `expires_in` (at most 7 days).
    pub fn presigned_url(&self, key: &str, expires_in: Duration) -> anyhow::Result<String> {
        let bucket = self.bucket_handle()?;
        let credentials = self.credentials();
        let mut action = bucket.get_object(Some(&credentials), key);
        action
            .query_mut()
            .insert("response-content-type", "text/html; charset=utf-8");
        Ok(action
            .sign(expires_in.min(Duration::from_secs(MAX_PRESIGN_SECS)))
            .to_string())
    }

    pub async fn put(
        &self,
        http: &reqwest::Client,
        key: &str,
        body: Vec<u8>,
        visibility: Visibility,
    ) -> anyhow::Result<()> {
        let bucket = self.bucket_handle()?;
        let credentials = self.credentials();
        let url = bucket
            .put_object(Some(&credentials), key)
            .sign(Duration::from_secs(600));
        let cache = match visibility {
            Visibility::Public => "public, max-age=300",
            Visibility::Private => "private, no-cache",
        };
        let response = send_idempotent(|| {
            http.put(url.clone())
                .header(reqwest::header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(reqwest::header::CACHE_CONTROL, cache)
                .body(body.clone())
        })
        .await
        .map_err(|e| anyhow::anyhow!("Could not reach {}: {}", self.endpoint, describe(&e)))?;
        check(response, "Upload").await
    }

    pub async fn delete(&self, http: &reqwest::Client, key: &str) -> anyhow::Result<()> {
        let bucket = self.bucket_handle()?;
        let credentials = self.credentials();
        let url = bucket
            .delete_object(Some(&credentials), key)
            .sign(Duration::from_secs(600));
        let response = send_idempotent(|| http.delete(url.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("Could not reach {}: {}", self.endpoint, describe(&e)))?;
        check(response, "Delete").await
    }

    /// Uploads, reads and deletes test objects to check credentials, permissions
    /// and that public/private links behave as the mode promises.
    pub async fn test(&self, http: &reqwest::Client) -> Vec<S3Check> {
        let mut checks = Vec::new();
        if let Err(e) = self.validate() {
            checks.push(S3Check::fail("Configuration", e.to_string()));
            return checks;
        }
        let id = format!(
            "scratchpad-connection-test-{}",
            uuid::Uuid::new_v4().simple()
        );
        let body = b"<!doctype html><title>Scratchpad</title>Scratchpad connection test".to_vec();

        for visibility in [Visibility::Public, Visibility::Private] {
            if !self.mode.allows(visibility) {
                continue;
            }
            let label = match visibility {
                Visibility::Public => "public",
                Visibility::Private => "private",
            };
            let key = Self::object_key(visibility, &id);
            if let Err(e) = self.put(http, &key, body.clone(), visibility).await {
                checks.push(S3Check::fail(
                    format!("Upload {label} object"),
                    format!("{e:#}"),
                ));
                continue;
            }
            checks.push(S3Check::ok(format!("Upload {label} object"), key.clone()));

            match visibility {
                Visibility::Public => {
                    let url = self.public_url(&key).unwrap_or_default();
                    checks.push(match fetch_status(http, &url).await {
                        Ok(200) => S3Check::ok("Public link opens", url),
                        Ok(status) => S3Check::fail(
                            "Public link opens",
                            format!(
                                "{url} returned HTTP {status}. Make {PUBLIC_PREFIX} publicly readable \
                                 (bucket policy, public bucket or CDN)."
                            ),
                        ),
                        Err(e) => S3Check::fail("Public link opens", format!("{url}: {e}")),
                    });
                }
                Visibility::Private => {
                    let signed = self
                        .presigned_url(&key, Duration::from_secs(300))
                        .unwrap_or_default();
                    checks.push(match fetch_status(http, &signed).await {
                        Ok(200) => S3Check::ok("Presigned link opens", "HTTP 200"),
                        Ok(status) => S3Check::fail(
                            "Presigned link opens",
                            format!("HTTP {status}. Check the region and the system clock."),
                        ),
                        Err(e) => S3Check::fail("Presigned link opens", e),
                    });
                    let mut urls = vec![self.direct_url(&key).map(|u| u.to_string()).ok()];
                    if !self.cdn_domain.trim().is_empty() {
                        urls.push(self.public_url(&key).ok());
                    }
                    for url in urls.into_iter().flatten() {
                        checks.push(match fetch_status(http, &url).await {
                            Ok(200) => S3Check::fail(
                                "Private objects stay private",
                                format!(
                                    "{url} is publicly readable. Only {PUBLIC_PREFIX} should be public."
                                ),
                            ),
                            Ok(status) => S3Check::ok(
                                "Private objects stay private",
                                format!("{url} → HTTP {status}"),
                            ),
                            // Unreachable without a signature is private enough.
                            Err(e) => S3Check::ok("Private objects stay private", e),
                        });
                    }
                }
            }
            if let Err(e) = self.delete(http, &key).await {
                checks.push(S3Check::fail(
                    format!("Delete {label} object"),
                    format!("{e:#}"),
                ));
            } else {
                checks.push(S3Check::ok(format!("Delete {label} object"), ""));
            }
        }
        checks
    }
}

/// One step of an S3 connection test.
#[derive(Debug, Clone, Serialize)]
pub struct S3Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

impl S3Check {
    fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: true,
            detail: detail.into(),
        }
    }

    fn fail(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: false,
            detail: detail.into(),
        }
    }
}

/// Servers may close pooled keep-alive connections between requests. S3 PUT and
/// DELETE are idempotent, so a request that failed before getting a response is
/// retried once.
async fn send_idempotent(
    request: impl Fn() -> reqwest::RequestBuilder,
) -> reqwest::Result<reqwest::Response> {
    match request().send().await {
        Err(e) if !e.is_timeout() && (e.is_connect() || e.is_request()) => {
            tracing::debug!(error = %describe(&e), "retrying S3 request");
            request().send().await
        }
        result => result,
    }
}

async fn fetch_status(http: &reqwest::Client, url: &str) -> Result<u16, String> {
    http.get(url)
        .send()
        .await
        .map(|r| r.status().as_u16())
        .map_err(|e| describe(&e))
}

async fn check(response: reqwest::Response, action: &str) -> anyhow::Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let start = body.find(&open)? + open.len();
        let end = body[start..].find(&format!("</{name}>"))?;
        Some(body[start..start + end].to_string())
    };
    match (tag("Code"), tag("Message")) {
        (Some(code), Some(message)) => {
            anyhow::bail!(
                "{action} failed: {code}: {message} (HTTP {})",
                status.as_u16()
            )
        }
        (Some(code), None) => anyhow::bail!("{action} failed: {code} (HTTP {})", status.as_u16()),
        _ => anyhow::bail!("{action} failed: HTTP {}", status.as_u16()),
    }
}

/// The error and its causes. reqwest includes the request URL, which for S3 is a
/// presigned URL: its query (signature, access key id) is a credential and is removed.
fn describe(error: &reqwest::Error) -> String {
    use std::error::Error;
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        message = format!("{message}: {inner}");
        source = inner.source();
    }
    match error.url() {
        Some(url) if url.query().is_some() => {
            let mut clean = url.clone();
            clean.set_query(None);
            message.replace(url.as_str(), clean.as_str())
        }
        _ => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> S3Config {
        S3Config {
            id: "x".into(),
            name: "Test".into(),
            endpoint: "https://s3.example.com".into(),
            region: "us-east-1".into(),
            bucket: "mails".into(),
            access_key: "AKID".into(),
            secret_key: "secret".into(),
            cdn_domain: String::new(),
            mode: S3Mode::Both,
            path_style: true,
            storage: CredentialStorage::Local,
        }
    }

    #[test]
    fn urls() {
        let mut c = config();
        let key = S3Config::object_key(Visibility::Public, "abc");
        assert_eq!(key, "email/shares/abc.html");
        assert_eq!(
            c.public_url(&key).unwrap(),
            "https://s3.example.com/mails/email/shares/abc.html"
        );
        c.path_style = false;
        assert_eq!(
            c.public_url(&key).unwrap(),
            "https://mails.s3.example.com/email/shares/abc.html"
        );
        c.cdn_domain = "share.example.com/".into();
        assert_eq!(
            c.public_url(&key).unwrap(),
            "https://share.example.com/email/shares/abc.html"
        );
        let signed = c
            .presigned_url(
                &S3Config::object_key(Visibility::Private, "abc"),
                Duration::from_secs(3600),
            )
            .unwrap();
        // Presigned links always go to the endpoint, never the CDN.
        assert!(signed.starts_with("https://mails.s3.example.com/email/private/abc.html?"));
        assert!(signed.contains("X-Amz-Expires=3600"));
        assert!(signed.contains("X-Amz-Signature="));
        assert!(!format!("{c:?}").contains("secret"));
    }

    #[test]
    fn endpoint_with_path_keeps_it() {
        let mut c = config();
        c.endpoint = "http://localhost:9000/storage".into();
        assert_eq!(
            c.direct_url("email/shares/a.html").unwrap().as_str(),
            "http://localhost:9000/storage/mails/email/shares/a.html"
        );
    }

    #[tokio::test]
    async fn errors_never_contain_signatures() {
        crate::tls::install_crypto_provider();
        let mut c = config();
        // Nothing listens on port 1.
        c.endpoint = "http://127.0.0.1:1".into();
        let error = c
            .put(
                &reqwest::Client::new(),
                "email/shares/x.html",
                vec![],
                Visibility::Public,
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("127.0.0.1:1"), "{error}");
        assert!(!error.contains("X-Amz-Signature"), "{error}");
        assert!(!error.contains("AKID"), "{error}");
    }

    #[test]
    fn presigned_expiry_is_capped() {
        let signed = config()
            .presigned_url("email/private/a.html", Duration::from_secs(30 * 24 * 3600))
            .unwrap();
        assert!(signed.contains(&format!("X-Amz-Expires={MAX_PRESIGN_SECS}")));
        assert!(signed.contains("X-Amz-Credential=AKID%2F"));
        assert!(!signed.contains("secret"));
    }

    #[test]
    fn validation() {
        let mut c = config();
        assert!(c.validate().is_ok());
        c.endpoint = "ftp://x".into();
        assert!(c.validate().is_err());
        c = config();
        c.endpoint = "s3.example.com".into();
        assert_eq!(
            c.direct_url("k").unwrap().as_str(),
            "https://s3.example.com/mails/k"
        );
        c.secret_key.clear();
        assert!(c.validate().is_err());
        c = config();
        c.name = "  ".into();
        assert!(c.validate().is_err());
        c = config();
        c.bucket.clear();
        assert!(c.validate().is_err());
        // The redacted form never carries the secret.
        let info = config().redacted();
        assert!(info.has_secret && info.config.secret_key.is_empty());
        assert!(!serde_json::to_string(&info).unwrap().contains("\"secret\""));
    }

    #[test]
    fn modes() {
        assert!(S3Mode::Public.allows(Visibility::Public));
        assert!(!S3Mode::Public.allows(Visibility::Private));
        assert!(!S3Mode::Private.allows(Visibility::Public));
        assert!(S3Mode::Both.allows(Visibility::Private));
    }
}
