//! Storage for S3 credentials.
//!
//! * **Local** configs live in `credentials.json` in the data directory (owner-only
//!   permissions on Unix) and work fully offline.
//! * **Synced** configs live in an encrypted vault (`vault.json`): JSON sealed with
//!   XChaCha20-Poly1305 under a key derived from the master password with Argon2id.
//!   Only that sealed blob is handed to the [`SyncBackend`], so the backend never
//!   sees credentials or the password. The vault must be unlocked (per session) to
//!   use or edit synced configs.
//!
//! The cloud backend is a stub for now ([`StubSyncBackend`]); a client for the
//! cloud auth service will implement [`SyncBackend`] later.

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use anyhow::Context;
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use futures::future::BoxFuture;
use rand::Rng;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::s3::{CredentialStorage, S3Config};
use crate::write_private;

const LOCAL_FILE: &str = "credentials.json";
const VAULT_FILE: &str = "vault.json";
const AAD: &[u8] = b"scratchpad vault v1";
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Upper bounds for KDF parameters read from a vault blob. Blobs can come from the
/// sync backend, so hostile parameters must not exhaust memory or CPU.
pub const MAX_KDF_MEMORY_KIB: u32 = 1024 * 1024;
pub const MAX_KDF_ITERATIONS: u32 = 64;
pub const MAX_KDF_PARALLELISM: u32 = 16;
const MAX_SALT_LEN: usize = 64;

/// Argon2id cost parameters, stored with each vault so they can be raised later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KdfParams {
    pub algorithm: String,
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    pub salt: String,
}

impl KdfParams {
    fn new(cost: KdfCost) -> Self {
        let mut salt = [0u8; 16];
        rand::rng().fill_bytes(&mut salt);
        Self {
            algorithm: "argon2id".into(),
            memory_kib: cost.memory_kib,
            iterations: cost.iterations,
            parallelism: 1,
            salt: B64.encode(salt),
        }
    }

    fn derive(&self, password: &str) -> anyhow::Result<Zeroizing<[u8; 32]>> {
        if self.algorithm != "argon2id" {
            anyhow::bail!("unsupported key derivation {}", self.algorithm);
        }
        if self.memory_kib > MAX_KDF_MEMORY_KIB
            || self.iterations > MAX_KDF_ITERATIONS
            || self.parallelism > MAX_KDF_PARALLELISM
        {
            anyhow::bail!("the vault's key derivation parameters are out of range");
        }
        let params = Params::new(self.memory_kib, self.iterations, self.parallelism, Some(32))
            .map_err(|e| anyhow::anyhow!("invalid key derivation parameters: {e}"))?;
        let salt = B64.decode(&self.salt).context("invalid vault salt")?;
        if salt.len() > MAX_SALT_LEN {
            anyhow::bail!("invalid vault salt");
        }
        let mut key = Zeroizing::new([0u8; 32]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(password.as_bytes(), &salt, key.as_mut())
            .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
        Ok(key)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KdfCost {
    pub memory_kib: u32,
    pub iterations: u32,
}

impl Default for KdfCost {
    /// 64 MiB, 3 passes: well above OWASP's minimum for Argon2id.
    fn default() -> Self {
        Self {
            memory_kib: 64 * 1024,
            iterations: 3,
        }
    }
}

/// The encrypted vault, exactly as stored on disk and synced to the backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultBlob {
    pub version: u32,
    pub kdf: KdfParams,
    pub cipher: String,
    pub nonce: String,
    pub ciphertext: String,
    pub updated_at: i64,
}

impl VaultBlob {
    fn seal(kdf: &KdfParams, key: &[u8; 32], configs: &[S3Config]) -> anyhow::Result<Self> {
        let mut plaintext = Zeroizing::new(serde_json::to_vec(configs)?);
        let mut nonce = [0u8; 24];
        rand::rng().fill_bytes(&mut nonce);
        let ciphertext = XChaCha20Poly1305::new(key.into())
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: AAD,
                },
            )
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;
        plaintext.zeroize();
        Ok(Self {
            version: 1,
            kdf: kdf.clone(),
            cipher: "xchacha20poly1305".into(),
            nonce: B64.encode(nonce),
            ciphertext: B64.encode(ciphertext),
            updated_at: chrono::Utc::now().timestamp(),
        })
    }

    fn open(&self, key: &[u8; 32]) -> anyhow::Result<Vec<S3Config>> {
        if self.version != 1 || self.cipher != "xchacha20poly1305" {
            anyhow::bail!("this vault was created by a newer version of Scratchpad");
        }
        let nonce = B64.decode(&self.nonce).context("corrupt vault")?;
        if nonce.len() != 24 {
            anyhow::bail!("corrupt vault");
        }
        let ciphertext = B64.decode(&self.ciphertext).context("corrupt vault")?;
        let plaintext = Zeroizing::new(
            XChaCha20Poly1305::new(key.into())
                .decrypt(
                    &XNonce::try_from(nonce.as_slice()).context("corrupt vault")?,
                    Payload {
                        msg: &ciphertext,
                        aad: AAD,
                    },
                )
                .map_err(|_| anyhow::anyhow!("Wrong master password (or the vault is corrupt)"))?,
        );
        serde_json::from_slice(&plaintext).context("corrupt vault")
    }

    fn unlock(&self, password: &str) -> anyhow::Result<Unlocked> {
        let key = self.kdf.derive(password)?;
        let configs = self.open(&key)?;
        Ok(Unlocked {
            key,
            kdf: self.kdf.clone(),
            configs,
        })
    }
}

struct Unlocked {
    key: Zeroizing<[u8; 32]>,
    kdf: KdfParams,
    configs: Vec<S3Config>,
}

impl Drop for Unlocked {
    /// Wipes the decrypted secrets when the vault is locked.
    fn drop(&mut self) {
        for config in &mut self.configs {
            config.secret_key.zeroize();
        }
    }
}

impl Unlocked {
    fn seal(&self) -> anyhow::Result<VaultBlob> {
        VaultBlob::seal(&self.kdf, &self.key, &self.configs)
    }
}

/// Where synced vaults go. Implemented by the cloud auth service client.
pub trait SyncBackend: Send + Sync {
    fn info(&self) -> SyncBackendInfo;
    fn push(&self, blob: VaultBlob) -> BoxFuture<'static, anyhow::Result<()>>;
    fn pull(&self) -> BoxFuture<'static, anyhow::Result<Option<VaultBlob>>>;
    fn delete(&self) -> BoxFuture<'static, anyhow::Result<()>>;
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncBackendInfo {
    pub name: String,
    /// Signed in to the cloud service.
    pub connected: bool,
    pub note: String,
}

/// Placeholder for the cloud backend: mirrors the encrypted vault into a local
/// directory so the whole flow (sync, restore on another device) can be exercised.
pub struct StubSyncBackend {
    dir: PathBuf,
}

impl StubSyncBackend {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(VAULT_FILE)
    }
}

impl SyncBackend for StubSyncBackend {
    fn info(&self) -> SyncBackendInfo {
        SyncBackendInfo {
            name: "Cloud sync (preview)".into(),
            connected: false,
            note: format!(
                "Cloud sync isn't available yet. The encrypted vault is mirrored to {} instead.",
                self.dir.display()
            ),
        }
    }

    fn push(&self, blob: VaultBlob) -> BoxFuture<'static, anyhow::Result<()>> {
        let path = self.path();
        Box::pin(async move {
            let bytes = serde_json::to_vec_pretty(&blob)?;
            tokio::task::spawn_blocking(move || write_private(&path, &bytes)).await?
        })
    }

    fn pull(&self) -> BoxFuture<'static, anyhow::Result<Option<VaultBlob>>> {
        let path = self.path();
        Box::pin(async move {
            match tokio::fs::read(&path).await {
                Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
    }

    fn delete(&self) -> BoxFuture<'static, anyhow::Result<()>> {
        let path = self.path();
        Box::pin(async move {
            match tokio::fs::remove_file(&path).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            }
        })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncState {
    pub last_sync_at: Option<i64>,
    pub last_error: Option<String>,
}

/// What the UIs show about the vault.
#[derive(Debug, Clone, Serialize)]
pub struct VaultStatus {
    /// A master password has been set.
    pub exists: bool,
    pub unlocked: bool,
    pub synced_configs: Option<usize>,
    pub updated_at: Option<i64>,
    pub backend: SyncBackendInfo,
    pub sync: SyncState,
}

pub struct Vault {
    dir: PathBuf,
    cost: KdfCost,
    local: RwLock<Vec<S3Config>>,
    unlocked: RwLock<Option<Unlocked>>,
    backend: Box<dyn SyncBackend>,
    sync: RwLock<SyncState>,
    /// Serialises read-modify-write changes to the files.
    edit: Mutex<()>,
}

impl Vault {
    pub fn open(
        dir: impl Into<PathBuf>,
        backend: Box<dyn SyncBackend>,
        cost: KdfCost,
    ) -> anyhow::Result<Self> {
        let dir = dir.into();
        let path = dir.join(LOCAL_FILE);
        let local = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(configs) => configs,
                Err(error) => {
                    // Keep the app usable, and the broken file for manual recovery.
                    let backup = dir.join(format!("{LOCAL_FILE}.corrupt"));
                    std::fs::rename(&path, &backup)
                        .with_context(|| format!("{} is corrupt: {error}", path.display()))?;
                    tracing::warn!(%error, backup = %backup.display(), "S3 credentials file is corrupt, moved aside");
                    vec![]
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self {
            dir,
            cost,
            local: RwLock::new(local),
            unlocked: RwLock::new(None),
            backend,
            sync: RwLock::new(SyncState::default()),
            edit: Mutex::new(()),
        })
    }

    fn vault_path(&self) -> PathBuf {
        self.dir.join(VAULT_FILE)
    }

    fn read_blob(&self) -> anyhow::Result<Option<VaultBlob>> {
        match std::fs::read(self.vault_path()) {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).context("corrupt vault file")?,
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write_blob(&self, blob: &VaultBlob) -> anyhow::Result<()> {
        write_private(&self.vault_path(), &serde_json::to_vec_pretty(blob)?)
    }

    fn save_local(&self, configs: &[S3Config]) -> anyhow::Result<()> {
        write_private(
            &self.dir.join(LOCAL_FILE),
            &serde_json::to_vec_pretty(configs)?,
        )
    }

    pub fn status(&self) -> VaultStatus {
        let blob = self.read_blob().ok().flatten();
        let unlocked = self.unlocked.read().unwrap();
        VaultStatus {
            exists: blob.is_some(),
            unlocked: unlocked.is_some(),
            synced_configs: unlocked.as_ref().map(|u| u.configs.len()),
            updated_at: blob.map(|b| b.updated_at),
            backend: self.backend.info(),
            sync: self.sync.read().unwrap().clone(),
        }
    }

    /// Local configs plus the synced ones when the vault is unlocked.
    pub fn configs(&self) -> Vec<S3Config> {
        let mut all = self.local.read().unwrap().clone();
        if let Some(unlocked) = self.unlocked.read().unwrap().as_ref() {
            all.extend(unlocked.configs.iter().cloned());
        }
        all
    }

    pub fn get(&self, id: &str) -> Option<S3Config> {
        self.configs().into_iter().find(|c| c.id == id)
    }

    /// Adds or updates a config. An empty secret keeps the stored one. Returns the
    /// saved config and whether the vault changed (and should be synced).
    pub fn save(&self, mut config: S3Config) -> anyhow::Result<(S3Config, bool)> {
        let _edit = self.edit.lock().unwrap_or_else(|e| e.into_inner());
        if config.id.is_empty() {
            config.id = uuid::Uuid::new_v4().simple().to_string();
        }
        let previous = self.get(&config.id);
        if config.secret_key.is_empty() {
            if let Some(previous) = &previous {
                config.secret_key = previous.secret_key.clone();
            }
        }
        config.validate()?;
        if self
            .configs()
            .iter()
            .any(|c| c.id != config.id && c.name.trim().eq_ignore_ascii_case(config.name.trim()))
        {
            anyhow::bail!(
                "An S3 config named \"{}\" already exists",
                config.name.trim()
            );
        }
        let was_synced = previous
            .as_ref()
            .is_some_and(|p| p.storage == CredentialStorage::Synced);
        if config.storage == CredentialStorage::Synced || was_synced {
            self.require_unlocked()?;
        }

        let mut local = self.local.read().unwrap().clone();
        local.retain(|c| c.id != config.id);
        let mut unlocked = self.unlocked.write().unwrap();
        if let Some(unlocked) = unlocked.as_mut() {
            unlocked.configs.retain(|c| c.id != config.id);
        }
        match config.storage {
            CredentialStorage::Local => local.push(config.clone()),
            CredentialStorage::Synced => unlocked
                .as_mut()
                .expect("checked above")
                .configs
                .push(config.clone()),
        }
        self.save_local(&local)?;
        *self.local.write().unwrap() = local;
        let vault_changed = config.storage == CredentialStorage::Synced || was_synced;
        if vault_changed {
            self.write_blob(&unlocked.as_ref().expect("checked above").seal()?)?;
        }
        Ok((config, vault_changed))
    }

    /// Removes a config. Returns whether the vault changed.
    pub fn delete(&self, id: &str) -> anyhow::Result<bool> {
        let _edit = self.edit.lock().unwrap_or_else(|e| e.into_inner());
        let mut local = self.local.read().unwrap().clone();
        let before = local.len();
        local.retain(|c| c.id != id);
        if local.len() != before {
            self.save_local(&local)?;
            *self.local.write().unwrap() = local;
            return Ok(false);
        }
        let mut unlocked = self.unlocked.write().unwrap();
        let Some(unlocked) = unlocked.as_mut() else {
            anyhow::bail!("S3 config not found (unlock the vault to manage synced configs)");
        };
        let before = unlocked.configs.len();
        unlocked.configs.retain(|c| c.id != id);
        if unlocked.configs.len() == before {
            anyhow::bail!("S3 config not found");
        }
        self.write_blob(&unlocked.seal()?)?;
        Ok(true)
    }

    fn require_unlocked(&self) -> anyhow::Result<()> {
        if self.unlocked.read().unwrap().is_some() {
            return Ok(());
        }
        if self.vault_path().exists() {
            anyhow::bail!("Unlock the vault with your master password first");
        }
        anyhow::bail!("Set a master password to sync credentials")
    }

    /// Sets the master password and creates an empty vault. Blocking (key derivation).
    pub fn create(&self, password: &str) -> anyhow::Result<()> {
        check_password(password)?;
        let _edit = self.edit.lock().unwrap_or_else(|e| e.into_inner());
        if self.vault_path().exists() {
            anyhow::bail!("A master password is already set");
        }
        let kdf = KdfParams::new(self.cost);
        let unlocked = Unlocked {
            key: kdf.derive(password)?,
            kdf,
            configs: vec![],
        };
        self.write_blob(&unlocked.seal()?)?;
        *self.unlocked.write().unwrap() = Some(unlocked);
        Ok(())
    }

    /// Blocking (key derivation).
    pub fn unlock(&self, password: &str) -> anyhow::Result<()> {
        let blob = self
            .read_blob()?
            .ok_or_else(|| anyhow::anyhow!("No master password has been set"))?;
        let unlocked = blob.unlock(password)?;
        *self.unlocked.write().unwrap() = Some(unlocked);
        Ok(())
    }

    pub fn lock(&self) {
        *self.unlocked.write().unwrap() = None;
    }

    /// Re-encrypts the vault under a new password (and a fresh salt). Blocking.
    pub fn change_password(&self, current: &str, new: &str) -> anyhow::Result<()> {
        check_password(new)?;
        let _edit = self.edit.lock().unwrap_or_else(|e| e.into_inner());
        let blob = self
            .read_blob()?
            .ok_or_else(|| anyhow::anyhow!("No master password has been set"))?;
        let mut old = blob.unlock(current)?;
        let kdf = KdfParams::new(self.cost);
        let unlocked = Unlocked {
            key: kdf.derive(new)?,
            kdf,
            configs: std::mem::take(&mut old.configs),
        };
        self.write_blob(&unlocked.seal()?)?;
        *self.unlocked.write().unwrap() = Some(unlocked);
        Ok(())
    }

    /// Forgets the master password and every synced config on this device.
    pub fn reset(&self) -> anyhow::Result<()> {
        let _edit = self.edit.lock().unwrap_or_else(|e| e.into_inner());
        self.lock();
        match std::fs::remove_file(self.vault_path()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Replaces the local vault with `blob` if `password` opens it. Blocking.
    pub fn restore(&self, blob: &VaultBlob, password: &str) -> anyhow::Result<usize> {
        let unlocked = blob.unlock(password)?;
        let _edit = self.edit.lock().unwrap_or_else(|e| e.into_inner());
        let count = unlocked.configs.len();
        // Synced configs must not shadow local ones with the same id.
        let local_ids: Vec<String> = self
            .local
            .read()
            .unwrap()
            .iter()
            .map(|c| c.id.clone())
            .collect();
        if unlocked.configs.iter().any(|c| local_ids.contains(&c.id)) {
            anyhow::bail!("The synced vault contains configs that also exist locally");
        }
        self.write_blob(blob)?;
        *self.unlocked.write().unwrap() = Some(unlocked);
        Ok(count)
    }

    /// The vault blob to push, if a vault exists.
    pub fn blob(&self) -> anyhow::Result<Option<VaultBlob>> {
        self.read_blob()
    }

    pub fn backend(&self) -> &dyn SyncBackend {
        self.backend.as_ref()
    }

    pub fn record_sync(&self, result: &anyhow::Result<()>) {
        let mut sync = self.sync.write().unwrap();
        match result {
            Ok(()) => {
                sync.last_sync_at = Some(chrono::Utc::now().timestamp());
                sync.last_error = None;
            }
            Err(e) => sync.last_error = Some(format!("{e:#}")),
        }
    }
}

fn check_password(password: &str) -> anyhow::Result<()> {
    if password.chars().count() < 8 {
        anyhow::bail!("Use a master password with at least 8 characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s3::S3Mode;
    use std::path::Path;

    const CHEAP: KdfCost = KdfCost {
        memory_kib: 64,
        iterations: 1,
    };

    fn dir() -> PathBuf {
        std::env::temp_dir().join(format!("scratchpad-vault-{}", uuid::Uuid::new_v4()))
    }

    fn vault(dir: &Path) -> Vault {
        Vault::open(
            dir,
            Box::new(StubSyncBackend::new(dir.join("cloud"))),
            CHEAP,
        )
        .unwrap()
    }

    fn config(name: &str, storage: CredentialStorage) -> S3Config {
        S3Config {
            id: String::new(),
            name: name.into(),
            endpoint: "https://s3.example.com".into(),
            region: "auto".into(),
            bucket: "b".into(),
            access_key: "AKID".into(),
            secret_key: "top-secret-key".into(),
            cdn_domain: String::new(),
            mode: S3Mode::Both,
            path_style: true,
            storage,
        }
    }

    #[test]
    fn local_and_synced_configs() {
        let dir = dir();
        let v = vault(&dir);
        let (local, synced) = v.save(config("local", CredentialStorage::Local)).unwrap();
        assert!(!synced);
        // Synced configs need a master password.
        assert!(v.save(config("cloud", CredentialStorage::Synced)).is_err());
        v.create("correct horse").unwrap();
        let (cloud, synced) = v.save(config("cloud", CredentialStorage::Synced)).unwrap();
        assert!(synced);
        assert_eq!(v.configs().len(), 2);

        // The vault file never contains the secret in clear text.
        let raw = std::fs::read_to_string(dir.join(VAULT_FILE)).unwrap();
        assert!(!raw.contains("top-secret-key") && !raw.contains("cloud"));

        // Editing with an empty secret keeps the stored one.
        let mut edit = cloud.clone();
        edit.secret_key.clear();
        edit.bucket = "other".into();
        assert_eq!(v.save(edit).unwrap().0.secret_key, "top-secret-key");

        // A fresh process sees only local configs until unlocked.
        let v2 = vault(&dir);
        assert_eq!(v2.configs().len(), 1);
        assert!(v2.unlock("wrong password").is_err());
        v2.unlock("correct horse").unwrap();
        let all = v2.configs();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all.iter().find(|c| c.id == cloud.id).unwrap().bucket,
            "other"
        );

        // Moving a config between storages.
        let mut moved = local.clone();
        moved.storage = CredentialStorage::Synced;
        v2.save(moved).unwrap();
        v2.lock();
        assert!(v2.configs().is_empty());

        v2.change_password("wrong password", "new password")
            .unwrap_err();
        v2.change_password("correct horse", "new password").unwrap();
        let v3 = vault(&dir);
        v3.unlock("new password").unwrap();
        assert_eq!(v3.configs().len(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn restore_from_backend() {
        let dir = dir();
        let backend_dir = dir.join("cloud");
        let v = vault(&dir);
        v.create("correct horse").unwrap();
        v.save(config("cloud", CredentialStorage::Synced)).unwrap();
        v.backend().push(v.blob().unwrap().unwrap()).await.unwrap();

        // Another device sharing the same backend.
        let other = dir.join("other");
        let v2 = Vault::open(&other, Box::new(StubSyncBackend::new(&backend_dir)), CHEAP).unwrap();
        let blob = v2.backend().pull().await.unwrap().unwrap();
        assert!(v2.restore(&blob, "nope nope").is_err());
        assert_eq!(v2.restore(&blob, "correct horse").unwrap(), 1);
        assert_eq!(v2.configs()[0].secret_key, "top-secret-key");
        std::fs::remove_dir_all(dir).ok();
    }

    fn sealed_blob(dir: &Path) -> VaultBlob {
        let v = vault(dir);
        v.create("correct horse").unwrap();
        v.save(config("cloud", CredentialStorage::Synced)).unwrap();
        v.blob().unwrap().unwrap()
    }

    #[test]
    fn hostile_kdf_parameters_are_rejected_without_running_them() {
        let dir = dir();
        let blob = sealed_blob(&dir);
        let other = vault(&dir.join("other"));
        for hostile in [
            KdfParams {
                memory_kib: u32::MAX,
                ..blob.kdf.clone()
            },
            KdfParams {
                iterations: u32::MAX,
                ..blob.kdf.clone()
            },
            KdfParams {
                parallelism: 1000,
                ..blob.kdf.clone()
            },
            KdfParams {
                salt: B64.encode([0u8; 4096]),
                ..blob.kdf.clone()
            },
            KdfParams {
                algorithm: "scrypt".into(),
                ..blob.kdf.clone()
            },
        ] {
            let started = std::time::Instant::now();
            let tampered = VaultBlob {
                kdf: hostile,
                ..blob.clone()
            };
            assert!(other.restore(&tampered, "correct horse").is_err());
            assert!(started.elapsed() < std::time::Duration::from_secs(2));
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupted_or_tampered_blobs_fail_cleanly() {
        let dir = dir();
        let blob = sealed_blob(&dir);
        let other = vault(&dir.join("other"));
        assert!(other.restore(&blob, "wrong password").is_err());

        let mut ciphertext = B64.decode(&blob.ciphertext).unwrap();
        ciphertext[0] ^= 1;
        let flipped = VaultBlob {
            ciphertext: B64.encode(&ciphertext),
            ..blob.clone()
        };
        assert!(other.restore(&flipped, "correct horse").is_err());
        for broken in [
            VaultBlob {
                nonce: "short".into(),
                ..blob.clone()
            },
            VaultBlob {
                ciphertext: "!!not base64!!".into(),
                ..blob.clone()
            },
            VaultBlob {
                version: 99,
                ..blob.clone()
            },
        ] {
            assert!(other.restore(&broken, "correct horse").is_err());
        }
        assert!(other.configs().is_empty());
        assert_eq!(other.restore(&blob, "correct horse").unwrap(), 1);

        // A vault file that isn't JSON is reported, not a panic.
        std::fs::write(dir.join(VAULT_FILE), b"{garbage").unwrap();
        let v = vault(&dir);
        assert!(v.unlock("correct horse").is_err());
        assert!(!v.status().exists);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn fresh_nonce_and_salt_every_time() {
        let dir = dir();
        let v = vault(&dir);
        v.create("correct horse").unwrap();
        let first = v.blob().unwrap().unwrap();
        v.save(config("a", CredentialStorage::Synced)).unwrap();
        let second = v.blob().unwrap().unwrap();
        assert_ne!(first.nonce, second.nonce);
        assert_eq!(first.kdf.salt, second.kdf.salt);
        v.change_password("correct horse", "battery staple")
            .unwrap();
        assert_ne!(v.blob().unwrap().unwrap().kdf.salt, first.kdf.salt);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupt_credentials_file_is_moved_aside() {
        let dir = dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOCAL_FILE), b"[{not json").unwrap();
        let v = vault(&dir);
        assert!(v.configs().is_empty());
        assert!(dir.join("credentials.json.corrupt").exists());
        v.save(config("local", CredentialStorage::Local)).unwrap();
        assert_eq!(vault(&dir).configs().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deleting_unknown_configs_fails() {
        let dir = dir();
        let v = vault(&dir);
        assert!(v.delete("nope").is_err());
        v.create("correct horse").unwrap();
        assert!(v.delete("nope").is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn credential_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir();
        std::fs::create_dir_all(&dir).unwrap();
        // A stale temporary file with loose permissions must not be reused.
        std::fs::write(dir.join("credentials.json.tmp"), b"").unwrap();
        std::fs::set_permissions(
            dir.join("credentials.json.tmp"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let v = vault(&dir);
        v.save(config("local", CredentialStorage::Local)).unwrap();
        v.create("correct horse").unwrap();
        for file in [LOCAL_FILE, VAULT_FILE] {
            let mode = std::fs::metadata(dir.join(file))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{file}");
        }
        assert!(!dir.join("credentials.json.tmp").exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
