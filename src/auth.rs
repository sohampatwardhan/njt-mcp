//! Per-API token acquisition.
//!
//! One username and password are shared. Each [`Api`] has its own token.
//! `NJT_TOKEN`, when set, is tried first on every API. A rejection falls back
//! to a disk token and then to a single-flight login. Token-only mode cannot
//! mint: the tool error says the token expired.

use std::collections::BTreeSet;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::api::Api;
use crate::client::{max_bytes_for, Client};
use crate::config::Credentials;
use crate::error::{redact_error, AppError};
use crate::store::Store;
use crate::upstream::{self, Classified};

/// Mints and caches one token per NJ Transit product.
///
/// # Examples
///
/// ```no_run
/// use njt_mcp::auth::Tokens;
/// use njt_mcp::client::Client;
/// use njt_mcp::store::Store;
/// use njt_mcp::{Api, Config};
/// # async fn demo() {
/// let config = Config::parse([
///     ("NJT_USERNAME", "replace-me"),
///     ("NJT_PASSWORD", "replace-me"),
///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
/// ])
/// .unwrap();
/// let store = Store::open(config.cache_dir()).unwrap();
/// let client = Client::new(&config).unwrap();
/// let tokens = Tokens::new(config.credentials().clone(), client, std::sync::Arc::new(store));
/// let _ = tokens.acquire(Api::TrainData, &[]).await;
/// # }
/// ```
pub struct Tokens {
    /// Parsed credentials. Debug redacts them.
    credentials: Credentials,
    /// HTTP client. Origins only.
    client: Client,
    /// Shared disk store.
    store: Arc<Store>,
    /// One gate per [`Api::index`], held for the whole mint.
    gates: [Mutex<()>; 4],
    /// APIs that have already rejected `NJT_TOKEN` in this process.
    env_rejected: Mutex<BTreeSet<Api>>,
    /// Sticky login failure text, per API, so a bad password is not retried.
    login_failed: Mutex<BTreeSet<Api>>,
}

impl Tokens {
    /// Build a manager. It does not call NJ Transit.
    ///
    /// # Arguments
    ///
    /// * `credentials` - token and/or username and password.
    /// * `client` - HTTP client.
    /// * `store` - disk cache shared with the service.
    ///
    /// # Returns
    ///
    /// A [`Tokens`] value.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use njt_mcp::auth::Tokens;
    /// use njt_mcp::client::Client;
    /// use njt_mcp::store::Store;
    /// use njt_mcp::Config;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-tokens-{}", std::process::id()));
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", dir.to_str().unwrap()),
    /// ])
    /// .unwrap();
    /// let store = Arc::new(Store::open(config.cache_dir()).unwrap());
    /// let client = Client::new(&config).unwrap();
    /// let tokens = Tokens::new(config.credentials().clone(), client, store);
    /// assert!(!tokens.can_mint());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn new(credentials: Credentials, client: Client, store: Arc<Store>) -> Self {
        Self {
            credentials,
            client,
            store,
            gates: [
                Mutex::new(()),
                Mutex::new(()),
                Mutex::new(()),
                Mutex::new(()),
            ],
            env_rejected: Mutex::new(BTreeSet::new()),
            login_failed: Mutex::new(BTreeSet::new()),
        }
    }

    /// Whether username and password are available.
    ///
    /// # Returns
    ///
    /// `true` when a rejected token can be replaced.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use njt_mcp::auth::Tokens;
    /// use njt_mcp::client::Client;
    /// use njt_mcp::store::Store;
    /// use njt_mcp::Config;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-tokens-mint-{}", std::process::id()));
    /// let config = Config::parse([
    ///     ("NJT_USERNAME", "replace-me"),
    ///     ("NJT_PASSWORD", "replace-me"),
    ///     ("NJT_CACHE_DIR", dir.to_str().unwrap()),
    /// ])
    /// .unwrap();
    /// let tokens = Tokens::new(
    ///     config.credentials().clone(),
    ///     Client::new(&config).unwrap(),
    ///     Arc::new(Store::open(&dir).unwrap()),
    /// );
    /// assert!(tokens.can_mint());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn can_mint(&self) -> bool {
        self.credentials.has_password()
    }

    /// Borrow credentials.
    ///
    /// # Returns
    ///
    /// The credentials supplied at construction.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use njt_mcp::auth::Tokens;
    /// use njt_mcp::client::Client;
    /// use njt_mcp::store::Store;
    /// use njt_mcp::Config;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-tokens-cred-{}", std::process::id()));
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", dir.to_str().unwrap()),
    /// ])
    /// .unwrap();
    /// let tokens = Tokens::new(
    ///     config.credentials().clone(),
    ///     Client::new(&config).unwrap(),
    ///     Arc::new(Store::open(&dir).unwrap()),
    /// );
    /// assert_eq!(tokens.credentials().token(), Some("replace-me"));
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// Return a token that is not in `rejected`.
    ///
    /// The lock for `api` is held through a mint, so concurrent callers share
    /// one login. Disk tokens are used only after `NJT_TOKEN` is rejected or
    /// absent. `NJT_TOKEN` itself is never written to disk.
    ///
    /// # Arguments
    ///
    /// * `api` - product to authenticate.
    /// * `rejected` - tokens already refused by this call.
    ///
    /// # Returns
    ///
    /// A token string.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::token_expired`] in token-only mode once the supplied
    /// token is rejected. Returns [`AppError::mint_cap`] when the local login
    /// cap is exhausted. Returns [`AppError::login_failed`] when NJ Transit
    /// rejects the username and password; that failure sticks for the process.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(tokens: &njt_mcp::auth::Tokens) -> Result<(), njt_mcp::AppError> {
    /// let token = tokens.acquire(njt_mcp::Api::TrainData, &[]).await?;
    /// let _ = token;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn acquire(&self, api: Api, rejected: &[String]) -> Result<String, AppError> {
        let _gate = self.gates[api.index()].lock().await;
        if self.login_failed.lock().await.contains(&api) {
            return Err(self.scrub(AppError::login_failed()));
        }
        if let Some(token) = self.credentials.token() {
            let banned = rejected.iter().any(|item| item == token);
            let env_rejected = self.env_rejected.lock().await.contains(&api);
            if !banned && !env_rejected {
                return Ok(token.to_string());
            }
            self.env_rejected.lock().await.insert(api);
        }
        if let Some(cached) = self.store.token(api) {
            if !rejected.iter().any(|item| item == &cached) {
                return Ok(cached);
            }
            self.store.forget_token(api)?;
        }
        if !self.can_mint() {
            return Err(self.scrub(AppError::token_expired(api)));
        }
        self.mint(api).await
    }

    /// Log in to one API and store the minted token.
    ///
    /// # Arguments
    ///
    /// * `api` - see the signature.
    ///
    /// # Returns
    ///
    /// The value described above.
    ///
    /// # Errors
    ///
    /// Returns an error when the operation cannot finish. Credential values are not included.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// Used by the surrounding module. Doctests cover the public wrappers.
    async fn mint(&self, api: Api) -> Result<String, AppError> {
        if self.store.mint_count(api) >= api.mint_cap() {
            return Err(AppError::mint_cap(api));
        }
        let username = self.credentials.username().unwrap_or("");
        let password = self.credentials.password().unwrap_or("");
        let url = self.client.url(api, api.login_path());
        let response = self
            .client
            .post_form(
                &url,
                &[("username", username), ("password", password)],
                max_bytes_for(api.login_path()),
            )
            .await
            .map_err(|error| self.scrub(error))?;
        self.store.record_mint(api)?;
        match upstream::classify(response.status, &response.bytes) {
            Classified::Data(bytes) => match upstream::parse_login(&bytes) {
                Ok(token) => {
                    self.store.remember_token(api, &token)?;
                    Ok(token)
                }
                Err(error) => {
                    if error.kind() == crate::error::ErrorKind::LoginFailed {
                        self.login_failed.lock().await.insert(api);
                    }
                    Err(self.scrub(error))
                }
            },
            other => {
                let error = upstream::into_error(other, api, api.login_path());
                if error.kind() == crate::error::ErrorKind::LoginFailed {
                    self.login_failed.lock().await.insert(api);
                }
                Err(self.scrub(error))
            }
        }
    }

    /// Strip credential values out of an error before it is returned.
    ///
    /// # Arguments
    ///
    /// * `error` - see the signature.
    ///
    /// # Returns
    ///
    /// The value described above.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// Used by the surrounding module. Doctests cover the public wrappers.
    fn scrub(&self, error: AppError) -> AppError {
        let mut secrets = self.credentials.secrets();
        for api in Api::ALL {
            if let Some(token) = self.store.token(api) {
                secrets.push(token);
            }
        }
        let refs: Vec<&str> = secrets.iter().map(String::as_str).collect();
        redact_error(error, &refs)
    }
}

impl std::fmt::Debug for Tokens {
    /// Helper `fmt`.
    ///
    /// # Arguments
    ///
    /// * `formatter` - see the signature.
    ///
    /// # Returns
    ///
    /// The value described above.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// Used by the surrounding module. Doctests cover the public wrappers.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Tokens")
            .field("mode", &self.credentials.mode())
            .finish()
    }
}
