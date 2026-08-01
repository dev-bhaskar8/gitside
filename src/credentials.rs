use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use secrecy::{ExposeSecret, SecretString};

use crate::config::ApiProvider;

const SERVICE: &str = "gitside";

trait CredentialBackend: Send + Sync {
    fn load(&self, provider: ApiProvider) -> Result<Option<String>>;
    fn store(&self, provider: ApiProvider, secret: &str) -> Result<()>;
    fn delete(&self, provider: ApiProvider) -> Result<()>;
}

struct NativeCredentialBackend;

impl CredentialBackend for NativeCredentialBackend {
    fn load(&self, provider: ApiProvider) -> Result<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, account(provider))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn store(&self, provider: ApiProvider, secret: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, account(provider))?
            .set_password(secret)
            .map_err(Into::into)
    }

    fn delete(&self, provider: ApiProvider) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, account(provider))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialStatus {
    Unknown,
    Missing,
    Stored,
    Environment(String),
    SessionOnly,
    Unavailable(String),
}

impl CredentialStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Unknown => "Not checked yet".into(),
            Self::Missing => "Not configured".into(),
            Self::Stored => "Stored in OS keychain".into(),
            Self::Environment(name) => format!("Available from {name}"),
            Self::SessionOnly => "Available for this session only".into(),
            Self::Unavailable(reason) => format!("Keychain unavailable · {reason}"),
        }
    }
}

#[derive(Clone)]
pub struct CredentialStore {
    session: Arc<Mutex<HashMap<ApiProvider, SecretString>>>,
    backend: Arc<dyn CredentialBackend>,
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self {
            session: Arc::new(Mutex::new(HashMap::new())),
            backend: Arc::new(NativeCredentialBackend),
        }
    }
}

impl CredentialStore {
    pub async fn resolve(
        &self,
        provider: ApiProvider,
        environment_name: Option<&str>,
    ) -> Result<Option<SecretString>> {
        if let Some(secret) = self.session_secret(provider)? {
            return Ok(Some(secret));
        }
        match self.load_persistent(provider).await {
            Ok(Some(secret)) => return Ok(Some(secret)),
            Ok(None) => {}
            Err(_) => {}
        }
        Ok(environment_name
            .and_then(|name| std::env::var(name).ok())
            .filter(|value| !value.is_empty())
            .map(SecretString::from))
    }

    pub async fn status(
        &self,
        provider: ApiProvider,
        environment_name: Option<&str>,
    ) -> CredentialStatus {
        if self.session_secret(provider).ok().flatten().is_some() {
            return CredentialStatus::SessionOnly;
        }
        match self.load_persistent(provider).await {
            Ok(Some(_)) => CredentialStatus::Stored,
            Ok(None) => environment_name
                .filter(|name| std::env::var_os(name).is_some())
                .map(|name| CredentialStatus::Environment(name.into()))
                .unwrap_or(CredentialStatus::Missing),
            Err(error) => environment_name
                .filter(|name| std::env::var_os(name).is_some())
                .map(|name| CredentialStatus::Environment(name.into()))
                .unwrap_or_else(|| CredentialStatus::Unavailable(short_error(&error))),
        }
    }

    pub async fn store(
        &self,
        provider: ApiProvider,
        secret: SecretString,
    ) -> Result<CredentialStatus> {
        let backend = self.backend.clone();
        let keychain_secret = secret.clone();
        let result = tokio::task::spawn_blocking(move || {
            backend.store(provider, keychain_secret.expose_secret())
        })
        .await
        .context("credential task failed")?;
        match result {
            Ok(()) => {
                self.session
                    .lock()
                    .map_err(|_| anyhow::anyhow!("credential memory lock is poisoned"))?
                    .remove(&provider);
                Ok(CredentialStatus::Stored)
            }
            Err(_) => {
                self.session
                    .lock()
                    .map_err(|_| anyhow::anyhow!("credential memory lock is poisoned"))?
                    .insert(provider, secret);
                Ok(CredentialStatus::SessionOnly)
            }
        }
    }

    pub async fn delete(&self, provider: ApiProvider) -> Result<CredentialStatus> {
        self.session
            .lock()
            .map_err(|_| anyhow::anyhow!("credential memory lock is poisoned"))?
            .remove(&provider);
        let backend = self.backend.clone();
        let result = tokio::task::spawn_blocking(move || backend.delete(provider))
            .await
            .context("credential task failed")?;
        result.context("failed to remove API key from the OS keychain")?;
        Ok(CredentialStatus::Missing)
    }

    fn session_secret(&self, provider: ApiProvider) -> Result<Option<SecretString>> {
        Ok(self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("credential memory lock is poisoned"))?
            .get(&provider)
            .cloned())
    }

    async fn load_persistent(&self, provider: ApiProvider) -> Result<Option<SecretString>> {
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || backend.load(provider))
            .await
            .context("credential task failed")?
            .context("failed to read the OS keychain")
            .map(|secret| secret.map(SecretString::from))
    }

    #[cfg(test)]
    fn with_backend(backend: Arc<dyn CredentialBackend>) -> Self {
        Self {
            session: Arc::new(Mutex::new(HashMap::new())),
            backend,
        }
    }
}

fn account(provider: ApiProvider) -> &'static str {
    match provider {
        ApiProvider::Openai => "api/openai",
        ApiProvider::Anthropic => "api/anthropic",
        ApiProvider::Gemini => "api/gemini",
        ApiProvider::Openrouter => "api/openrouter",
        ApiProvider::Compatible => "api/compatible",
    }
}

fn short_error(error: &anyhow::Error) -> String {
    let text = error.to_string();
    text.chars().take(80).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MockBackend {
        values: Mutex<HashMap<ApiProvider, String>>,
        fail_writes: bool,
    }

    impl CredentialBackend for MockBackend {
        fn load(&self, provider: ApiProvider) -> Result<Option<String>> {
            Ok(self.values.lock().unwrap().get(&provider).cloned())
        }

        fn store(&self, provider: ApiProvider, secret: &str) -> Result<()> {
            if self.fail_writes {
                anyhow::bail!("mock keychain unavailable");
            }
            self.values.lock().unwrap().insert(provider, secret.into());
            Ok(())
        }

        fn delete(&self, provider: ApiProvider) -> Result<()> {
            self.values.lock().unwrap().remove(&provider);
            Ok(())
        }
    }

    #[test]
    fn credential_labels_never_contain_secret_material() {
        assert_eq!(account(ApiProvider::Openai), "api/openai");
        assert_eq!(CredentialStatus::Stored.label(), "Stored in OS keychain");
        assert_eq!(
            CredentialStatus::Environment("OPENAI_API_KEY".into()).label(),
            "Available from OPENAI_API_KEY"
        );
    }

    #[tokio::test]
    async fn mock_backend_supports_store_resolve_and_delete() {
        let store = CredentialStore::with_backend(Arc::new(MockBackend::default()));
        assert_eq!(
            store
                .store(ApiProvider::Openai, SecretString::from("secret".to_owned()))
                .await
                .unwrap(),
            CredentialStatus::Stored
        );
        let loaded = store
            .resolve(ApiProvider::Openai, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.expose_secret(), "secret");
        store.delete(ApiProvider::Openai).await.unwrap();
        assert!(
            store
                .resolve(ApiProvider::Openai, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn failed_keychain_write_falls_back_to_zeroizing_session_memory() {
        let backend = MockBackend {
            fail_writes: true,
            ..MockBackend::default()
        };
        let store = CredentialStore::with_backend(Arc::new(backend));
        assert_eq!(
            store
                .store(ApiProvider::Gemini, SecretString::from("secret".to_owned()))
                .await
                .unwrap(),
            CredentialStatus::SessionOnly
        );
        assert_eq!(
            store
                .resolve(ApiProvider::Gemini, None)
                .await
                .unwrap()
                .unwrap()
                .expose_secret(),
            "secret"
        );
    }
}
