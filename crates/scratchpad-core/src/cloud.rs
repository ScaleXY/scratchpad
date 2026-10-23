//! Cloud sharing: S3 configs, the credential vault and exported emails uploaded to S3.

use std::sync::Arc;
use std::time::Duration;

use crate::model::{CloudShare, SharedEmail};
use crate::s3::{S3Check, S3Config, Visibility, MAX_PRESIGN_SECS};
use crate::vault::VaultStatus;
use crate::{Event, Scratchpad};
use zeroize::Zeroizing;

/// Default lifetime of presigned links.
pub const DEFAULT_EXPIRY_SECS: u64 = 24 * 3600;

/// A share to upload. The HTML is produced by the exporter (see `scratchpad-web`).
#[derive(Debug, Clone)]
pub struct PublishRequest {
    pub title: String,
    pub email_ids: Vec<String>,
    pub config_id: String,
    /// Defaults to what the config's mode allows (public first).
    pub visibility: Option<Visibility>,
    /// Lifetime of private links; defaults to a day, at most 7 days.
    pub expires_in_secs: Option<u64>,
}

fn expiry(secs: Option<u64>) -> Duration {
    Duration::from_secs(
        secs.unwrap_or(DEFAULT_EXPIRY_SECS)
            .clamp(60, MAX_PRESIGN_SECS),
    )
}

impl Scratchpad {
    // ---- S3 configs ------------------------------------------------------------

    /// Every usable S3 config, with secrets. Synced configs only while the vault is unlocked.
    pub fn s3_configs(&self) -> Vec<S3Config> {
        self.vault.configs()
    }

    pub fn s3_config(&self, id: &str) -> Option<S3Config> {
        self.vault.get(id)
    }

    /// Adds or updates a config; an empty secret keeps the stored one.
    pub async fn save_s3_config(self: &Arc<Self>, config: S3Config) -> anyhow::Result<S3Config> {
        let (saved, vault_changed) = self.vault.save(config)?;
        if vault_changed {
            let _ = self.sync_vault().await;
        }
        self.emit(Event::CloudChanged);
        Ok(saved)
    }

    pub async fn delete_s3_config(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        if self.vault.delete(id)? {
            let _ = self.sync_vault().await;
        }
        self.emit(Event::CloudChanged);
        Ok(())
    }

    /// Runs the upload/read/delete checks. An empty secret uses the stored config's.
    pub async fn test_s3_config(&self, mut config: S3Config) -> Vec<S3Check> {
        if config.secret_key.is_empty() {
            if let Some(stored) = self.vault.get(&config.id) {
                config.secret_key = stored.secret_key;
            }
        }
        config.test(&self.s3_http).await
    }

    // ---- vault ------------------------------------------------------------------

    pub fn vault_status(&self) -> VaultStatus {
        self.vault.status()
    }

    /// Key derivation is deliberately slow, so it runs off the async runtime.
    async fn vault_blocking<T: Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(&Scratchpad) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let scratchpad = self.clone();
        let result = tokio::task::spawn_blocking(move || f(&scratchpad)).await?;
        self.emit(Event::CloudChanged);
        result
    }

    /// Sets the master password (creates an empty, unlocked vault).
    pub async fn vault_create(self: &Arc<Self>, password: String) -> anyhow::Result<()> {
        let password = Zeroizing::new(password);
        self.vault_blocking(move |h| h.vault.create(&password))
            .await?;
        let _ = self.sync_vault().await;
        Ok(())
    }

    pub async fn vault_unlock(self: &Arc<Self>, password: String) -> anyhow::Result<()> {
        let password = Zeroizing::new(password);
        self.vault_blocking(move |h| h.vault.unlock(&password))
            .await
    }

    pub fn vault_lock(&self) {
        self.vault.lock();
        self.emit(Event::CloudChanged);
    }

    pub async fn vault_change_password(
        self: &Arc<Self>,
        current: String,
        new: String,
    ) -> anyhow::Result<()> {
        let (current, new) = (Zeroizing::new(current), Zeroizing::new(new));
        self.vault_blocking(move |h| h.vault.change_password(&current, &new))
            .await?;
        let _ = self.sync_vault().await;
        Ok(())
    }

    /// Pushes the encrypted vault to the sync backend now.
    pub async fn vault_sync_now(&self) -> anyhow::Result<()> {
        let result = self.sync_vault().await;
        self.emit(Event::CloudChanged);
        result
    }

    async fn sync_vault(&self) -> anyhow::Result<()> {
        let Some(blob) = self.vault.blob()? else {
            return Ok(());
        };
        let result = self.vault.backend().push(blob).await;
        if let Err(error) = &result {
            tracing::warn!(%error, "vault sync failed");
        }
        self.vault.record_sync(&result);
        result
    }

    /// Replaces this device's vault with the synced one. Returns the number of configs.
    pub async fn vault_restore(self: &Arc<Self>, password: String) -> anyhow::Result<usize> {
        let password = Zeroizing::new(password);
        let blob = self
            .vault
            .backend()
            .pull()
            .await?
            .ok_or_else(|| anyhow::anyhow!("Nothing has been synced yet"))?;
        let count = self
            .vault_blocking(move |h| h.vault.restore(&blob, &password))
            .await?;
        self.vault.record_sync(&Ok(()));
        Ok(count)
    }

    /// Forgets the master password and synced configs, here and in the backend.
    pub async fn vault_reset(&self) -> anyhow::Result<()> {
        self.vault.reset()?;
        let result = self.vault.backend().delete().await;
        self.emit(Event::CloudChanged);
        result
    }

    // ---- cloud shares ---------------------------------------------------------------

    pub fn cloud_shares(&self) -> anyhow::Result<Vec<CloudShare>> {
        self.store.cloud_shares()
    }

    pub fn cloud_share(&self, id: &str) -> anyhow::Result<Option<CloudShare>> {
        self.store.cloud_share(id)
    }

    fn share_config(&self, id: &str) -> anyhow::Result<S3Config> {
        self.vault.get(id).ok_or_else(|| {
            anyhow::anyhow!("S3 config not found. If it is synced, unlock the vault first.")
        })
    }

    /// Uploads an exported share to `email/shares/<uuid>.html` (public) or
    /// `email/private/<uuid>.html` (presigned link) and records it.
    pub async fn publish_cloud_share(
        &self,
        request: PublishRequest,
        html: Vec<u8>,
    ) -> anyhow::Result<CloudShare> {
        if request.email_ids.is_empty() {
            anyhow::bail!("Select at least one email to share");
        }
        let config = self.share_config(&request.config_id)?;
        let visibility = request
            .visibility
            .unwrap_or_else(|| config.mode.default_visibility());
        if !config.mode.allows(visibility) {
            anyhow::bail!(
                "\"{}\" is set up for {} links only",
                config.name,
                match config.mode.default_visibility() {
                    Visibility::Public => "public",
                    Visibility::Private => "private",
                }
            );
        }
        let id = uuid::Uuid::new_v4().to_string();
        let key = S3Config::object_key(visibility, &id);
        let size = html.len() as u64;
        config.put(&self.s3_http, &key, html, visibility).await?;

        let (url, expires_at) = Self::link(&config, &key, visibility, request.expires_in_secs)?;
        let emails = request
            .email_ids
            .iter()
            .map(|id| SharedEmail {
                id: id.clone(),
                subject: self
                    .store
                    .summary(id)
                    .ok()
                    .flatten()
                    .map(|s| s.subject)
                    .unwrap_or_default(),
            })
            .collect();
        let share = CloudShare {
            id,
            title: request.title,
            emails,
            config_id: config.id.clone(),
            config_name: config.name.clone(),
            object_key: key,
            visibility,
            url,
            expires_at,
            size,
            created_at: chrono::Utc::now().timestamp(),
        };
        self.store.upsert_cloud_share(&share)?;
        self.emit(Event::CloudChanged);
        Ok(share)
    }

    fn link(
        config: &S3Config,
        key: &str,
        visibility: Visibility,
        expires_in_secs: Option<u64>,
    ) -> anyhow::Result<(String, Option<i64>)> {
        Ok(match visibility {
            Visibility::Public => (config.public_url(key)?, None),
            Visibility::Private => {
                let ttl = expiry(expires_in_secs);
                (
                    config.presigned_url(key, ttl)?,
                    Some(chrono::Utc::now().timestamp() + ttl.as_secs() as i64),
                )
            }
        })
    }

    /// Creates a new presigned link for a private share. Earlier links keep working
    /// until they expire.
    pub fn renew_cloud_share(
        &self,
        id: &str,
        expires_in_secs: Option<u64>,
    ) -> anyhow::Result<CloudShare> {
        let mut share = self
            .store
            .cloud_share(id)?
            .ok_or_else(|| anyhow::anyhow!("Share not found"))?;
        if share.visibility != Visibility::Private {
            anyhow::bail!("Only private links expire");
        }
        let config = self.share_config(&share.config_id)?;
        let (url, expires_at) = Self::link(
            &config,
            &share.object_key,
            share.visibility,
            expires_in_secs,
        )?;
        share.url = url;
        share.expires_at = expires_at;
        self.store.upsert_cloud_share(&share)?;
        self.emit(Event::CloudChanged);
        Ok(share)
    }

    /// Deletes the uploaded file (unless `keep_object`) and forgets the share.
    pub async fn delete_cloud_share(&self, id: &str, keep_object: bool) -> anyhow::Result<()> {
        let share = self
            .store
            .cloud_share(id)?
            .ok_or_else(|| anyhow::anyhow!("Share not found"))?;
        if !keep_object {
            let config = self.share_config(&share.config_id)?;
            config.delete(&self.s3_http, &share.object_key).await?;
        }
        self.store.delete_cloud_share(id)?;
        self.emit(Event::CloudChanged);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s3::{CredentialStorage, S3Mode};
    use crate::Settings;

    #[test]
    fn expiry_is_clamped() {
        assert_eq!(expiry(None).as_secs(), DEFAULT_EXPIRY_SECS);
        assert_eq!(expiry(Some(0)).as_secs(), 60);
        assert_eq!(expiry(Some(u64::MAX)).as_secs(), MAX_PRESIGN_SECS);
        assert_eq!(expiry(Some(3600)).as_secs(), 3600);
    }

    fn config(mode: S3Mode) -> S3Config {
        S3Config {
            id: String::new(),
            name: format!("{mode:?}"),
            // Nothing listens here: validation must fail before any upload.
            endpoint: "http://127.0.0.1:1".into(),
            region: "auto".into(),
            bucket: "b".into(),
            access_key: "AKID".into(),
            secret_key: "secret".into(),
            cdn_domain: String::new(),
            mode,
            path_style: true,
            storage: CredentialStorage::Local,
        }
    }

    fn request(config_id: &str, visibility: Option<Visibility>) -> PublishRequest {
        PublishRequest {
            title: "t".into(),
            email_ids: vec!["x".into()],
            config_id: config_id.into(),
            visibility,
            expires_in_secs: None,
        }
    }

    #[tokio::test]
    async fn publish_validates_mode_and_selection() {
        let scratchpad = Scratchpad::in_memory(Settings::default()).unwrap();
        let public = scratchpad
            .save_s3_config(config(S3Mode::Public))
            .await
            .unwrap();
        let private = scratchpad
            .save_s3_config(config(S3Mode::Private))
            .await
            .unwrap();

        let error = scratchpad
            .publish_cloud_share(request(&public.id, Some(Visibility::Private)), vec![])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("public links only"), "{error}");
        let error = scratchpad
            .publish_cloud_share(request(&private.id, Some(Visibility::Public)), vec![])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("private links only"), "{error}");

        let mut empty = request(&public.id, None);
        empty.email_ids.clear();
        assert!(scratchpad.publish_cloud_share(empty, vec![]).await.is_err());
        let error = scratchpad
            .publish_cloud_share(request("missing", None), vec![])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not found"), "{error}");
        assert!(scratchpad.cloud_shares().unwrap().is_empty());
    }

    #[tokio::test]
    async fn only_private_shares_can_be_renewed() {
        let scratchpad = Scratchpad::in_memory(Settings::default()).unwrap();
        let both = scratchpad
            .save_s3_config(config(S3Mode::Both))
            .await
            .unwrap();
        let mut share = CloudShare {
            id: "s".into(),
            title: "t".into(),
            emails: vec![],
            config_id: both.id.clone(),
            config_name: both.name.clone(),
            object_key: S3Config::object_key(Visibility::Public, "s"),
            visibility: Visibility::Public,
            url: String::new(),
            expires_at: None,
            size: 0,
            created_at: 0,
        };
        scratchpad.store.upsert_cloud_share(&share).unwrap();
        assert!(scratchpad.renew_cloud_share("s", None).is_err());
        share.visibility = Visibility::Private;
        share.object_key = S3Config::object_key(Visibility::Private, "s");
        scratchpad.store.upsert_cloud_share(&share).unwrap();
        let renewed = scratchpad
            .renew_cloud_share("s", Some(10 * 24 * 3600))
            .unwrap();
        assert!(renewed
            .url
            .contains(&format!("X-Amz-Expires={MAX_PRESIGN_SECS}")));
        let ttl = renewed.expires_at.unwrap() - chrono::Utc::now().timestamp();
        assert!((MAX_PRESIGN_SECS as i64 - 5..=MAX_PRESIGN_SECS as i64).contains(&ttl));
        assert!(scratchpad.renew_cloud_share("missing", None).is_err());
    }
}
