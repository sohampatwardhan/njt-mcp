//! On-disk token, quota, schedule, and GTFS cache.
//!
//! `state.json` is mode `0600`. It holds minted tokens, so it must not be
//! logged. A file that cannot be parsed is renamed aside and replaced. The
//! parse error is discarded because serde can quote the file body.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::Api;
use crate::error::AppError;
use crate::limits::{
    today_service_day, BUS_MINT_CAP, LIVE_CALL_CAP, RAIL_MINT_CAP, SCHEDULE_FETCH_CAP,
};

/// Persisted counters and minted tokens for one cache directory.
///
/// # Examples
///
/// ```
/// # #[cfg(unix)]
/// # {
/// use njt_mcp::store::Store;
/// use njt_mcp::Api;
/// let dir = std::env::temp_dir().join(format!(
///     "njt-mcp-store-doc-{}",
///     std::process::id()
/// ));
/// let _ = std::fs::remove_dir_all(&dir);
/// let store = Store::open(&dir).unwrap();
/// store.remember_token(Api::TrainData, "replace-me").unwrap();
/// let mode = {
///     use std::os::unix::fs::PermissionsExt;
///     std::fs::metadata(dir.join("state.json")).unwrap().permissions().mode() & 0o777
/// };
/// assert_eq!(mode, 0o600);
/// assert_eq!(store.token(Api::TrainData).as_deref(), Some("replace-me"));
/// let _ = std::fs::remove_dir_all(&dir);
/// # }
/// ```
#[derive(Debug)]
pub struct Store {
    /// Cache directory, mode `0700` when this process created it.
    root: PathBuf,
    /// Mutable state. Poisoned locks are recovered.
    inner: std::sync::Mutex<State>,
    /// Set when a previous `state.json` could not be parsed.
    load_warning: Option<String>,
}

/// On-disk quota and token record. Debug output does not include token strings.
///
/// # Examples
///
/// Constructed inside this module.
#[derive(Clone, Serialize, Deserialize)]
struct State {
    /// Format version. Unknown versions are treated as unreadable.
    version: u32,
    /// New York service day the counters belong to.
    day: String,
    /// Minted tokens keyed by [`Api::as_str`].
    tokens: BTreeMap<String, String>,
    /// Login attempts keyed by [`Api::as_str`].
    mints: BTreeMap<String, u32>,
    /// Upstream `getStationSchedule` calls today.
    schedule_fetches: u32,
    /// Upstream live calls today.
    live_calls: u32,
    /// Paths blocked by an upstream daily-limit message.
    blocked: BTreeMap<String, String>,
}

impl Store {
    /// Create the directory and load `state.json`.
    ///
    /// # Arguments
    ///
    /// * `root` - cache directory. Created if missing.
    ///
    /// # Returns
    ///
    /// A store. Counters from a previous New York day are zeroed. Tokens are kept.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the directory cannot be created.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-open-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// assert!(store.token(njt_mcp::Api::GtfsRt).is_none());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn open(root: &Path) -> Result<Self, AppError> {
        fs::create_dir_all(root).map_err(|error| {
            AppError::new(format!("cache directory {}: {error}", root.display()))
                .hint("Set NJT_CACHE_DIR to a writable path.")
        })?;
        lock_down_dir(root);
        let path = root.join("state.json");
        let (state, load_warning) = load_state(&path);
        let store = Self {
            root: root.to_path_buf(),
            inner: std::sync::Mutex::new(state),
            load_warning,
        };
        store.with_mut(|state, _path| {
            let _ = roll_day(state);
        })?;
        Ok(store)
    }

    /// Static warning when the previous state file was discarded.
    ///
    /// # Returns
    ///
    /// A fixed sentence, or `None`. The sentence does not quote the file.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-warn-{}", std::process::id()));
    /// let _ = std::fs::remove_dir_all(&dir);
    /// std::fs::create_dir_all(&dir).unwrap();
    /// std::fs::write(dir.join("state.json"), b"not-json super-secret-token").unwrap();
    /// let store = Store::open(&dir).unwrap();
    /// let warning = store.load_warning().unwrap();
    /// assert!(!warning.contains("super-secret-token"));
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn load_warning(&self) -> Option<&str> {
        self.load_warning.as_deref()
    }

    /// Cache directory.
    ///
    /// # Returns
    ///
    /// The path passed to [`Store::open`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-root-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// assert_eq!(store.root(), dir.as_path());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Minted or previously cached token for one API.
    ///
    /// # Arguments
    ///
    /// * `api` - product key.
    ///
    /// # Returns
    ///
    /// The token, or `None`.
    ///
    /// # Panics
    ///
    /// This function does not panic. A poisoned mutex is recovered.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// use njt_mcp::Api;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-tok-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// assert!(store.token(Api::BusDv2).is_none());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn token(&self, api: Api) -> Option<String> {
        self.with_ref(|state| state.tokens.get(api.as_str()).cloned())
    }

    /// Whether a token is stored. Status JSON uses this instead of the token.
    ///
    /// # Arguments
    ///
    /// * `api` - product key.
    ///
    /// # Returns
    ///
    /// `true` when a token is present.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// use njt_mcp::Api;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-has-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.remember_token(Api::GtfsG2, "replace-me").unwrap();
    /// assert!(store.has_token(Api::GtfsG2));
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn has_token(&self, api: Api) -> bool {
        self.token(api).is_some()
    }

    /// Store a minted token and flush the file.
    ///
    /// # Arguments
    ///
    /// * `api` - product the token belongs to.
    /// * `token` - token value. It is written to disk and not logged.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the file cannot be written.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// use njt_mcp::Api;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-remember-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.remember_token(Api::GtfsRt, "replace-me").unwrap();
    /// assert_eq!(store.token(Api::GtfsRt).as_deref(), Some("replace-me"));
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn remember_token(&self, api: Api, token: &str) -> Result<(), AppError> {
        self.with_mut(|state, _path| {
            state
                .tokens
                .insert(api.as_str().to_string(), token.to_string());
        })
    }

    /// Drop a token if it is the one currently stored.
    ///
    /// # Arguments
    ///
    /// * `api` - product key.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the file cannot be written.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// use njt_mcp::Api;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-forget-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.remember_token(Api::TrainData, "replace-me").unwrap();
    /// store.forget_token(Api::TrainData).unwrap();
    /// assert!(store.token(Api::TrainData).is_none());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn forget_token(&self, api: Api) -> Result<(), AppError> {
        self.with_mut(|state, _path| {
            state.tokens.remove(api.as_str());
        })
    }

    /// Login attempts recorded for this service day.
    ///
    /// # Arguments
    ///
    /// * `api` - product key.
    ///
    /// # Returns
    ///
    /// The counter, or 0.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// use njt_mcp::Api;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-mints-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// assert_eq!(store.mint_count(Api::TrainData), 0);
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn mint_count(&self, api: Api) -> u32 {
        self.with_ref(|state| state.mints.get(api.as_str()).copied().unwrap_or(0))
    }

    /// Record one login HTTP response.
    ///
    /// # Arguments
    ///
    /// * `api` - product that was called.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the file cannot be written, or when the local cap is already reached.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// use njt_mcp::Api;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-record-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.record_mint(Api::GtfsRt).unwrap();
    /// assert_eq!(store.mint_count(Api::GtfsRt), 1);
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn record_mint(&self, api: Api) -> Result<(), AppError> {
        if self.mint_count(api) >= api.mint_cap() {
            return Err(AppError::mint_cap(api));
        }
        self.with_mut(|state, _path| {
            let entry = state.mints.entry(api.as_str().to_string()).or_insert(0);
            *entry = entry.saturating_add(1);
        })
    }

    /// Schedule fetches used today.
    ///
    /// # Returns
    ///
    /// The counter.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-sched-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// assert_eq!(store.schedule_fetches(), 0);
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn schedule_fetches(&self) -> u32 {
        self.with_ref(|state| state.schedule_fetches)
    }

    /// Reserve one `getStationSchedule` call.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::schedule_cap`] when today's counter is already at the cap.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-reserve-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.reserve_schedule().unwrap();
    /// assert_eq!(store.schedule_fetches(), 1);
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn reserve_schedule(&self) -> Result<(), AppError> {
        if self.schedule_fetches() >= SCHEDULE_FETCH_CAP {
            return Err(AppError::schedule_cap());
        }
        self.with_mut(|state, _path| {
            state.schedule_fetches = state.schedule_fetches.saturating_add(1);
        })
    }

    /// Live calls used today.
    ///
    /// # Returns
    ///
    /// The counter.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-live-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// assert_eq!(store.live_calls(), 0);
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn live_calls(&self) -> u32 {
        self.with_ref(|state| state.live_calls)
    }

    /// Reserve one live upstream call.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::live_cap`] when today's counter is at the cap.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-live2-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.reserve_live().unwrap();
    /// assert_eq!(store.live_calls(), 1);
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn reserve_live(&self) -> Result<(), AppError> {
        if self.live_calls() >= LIVE_CALL_CAP {
            return Err(AppError::live_cap());
        }
        self.with_mut(|state, _path| {
            state.live_calls = state.live_calls.saturating_add(1);
        })
    }

    /// Remember that a path returned a daily-limit error.
    ///
    /// # Arguments
    ///
    /// * `path` - absolute API path.
    /// * `message` - already-redacted error text.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the file cannot be written.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-block-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.block_path("/api/TrainData/getVehicleData", "daily limit").unwrap();
    /// assert!(store.blocked_path("/api/TrainData/getVehicleData").is_some());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn block_path(&self, path: &str, message: &str) -> Result<(), AppError> {
        self.with_mut(|state, _path| {
            state.blocked.insert(path.to_string(), message.to_string());
        })
    }

    /// Previous daily-limit text for a path, if this service day already saw one.
    ///
    /// # Arguments
    ///
    /// * `path` - absolute API path.
    ///
    /// # Returns
    ///
    /// The stored message, or `None`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-blocked-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// assert!(store.blocked_path("/api/TrainData/getVehicleData").is_none());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn blocked_path(&self, path: &str) -> Option<String> {
        self.with_ref(|state| state.blocked.get(path).cloned())
    }

    /// Read a schedule body cached for today's service day.
    ///
    /// # Arguments
    ///
    /// * `station` - sanitized station code.
    /// * `njt_only` - the `NJTOnly` flag that was requested.
    ///
    /// # Returns
    ///
    /// The cached JSON value, or `None` when this station has not been fetched today.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the file exists but cannot be read. Unreadable
    /// JSON is treated as a cache miss.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-read-sched-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// assert!(store.read_schedule("EX", false).unwrap().is_none());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn read_schedule(&self, station: &str, njt_only: bool) -> Result<Option<Value>, AppError> {
        let path = self.schedule_path(station, njt_only);
        if !path.exists() {
            return Ok(None);
        }
        let bytes =
            fs::read(&path).map_err(|error| AppError::new(format!("schedule cache: {error}")))?;
        Ok(serde_json::from_slice(&bytes).ok())
    }

    /// Write today's schedule body.
    ///
    /// # Arguments
    ///
    /// * `station` - sanitized station code.
    /// * `njt_only` - the flag used for the upstream call.
    /// * `body` - JSON to serve until the next New York midnight.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the file cannot be written.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-write-sched-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.write_schedule("EX", true, &serde_json::json!([])).unwrap();
    /// assert!(store.read_schedule("EX", true).unwrap().is_some());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn write_schedule(
        &self,
        station: &str,
        njt_only: bool,
        body: &Value,
    ) -> Result<(), AppError> {
        let path = self.schedule_path(station, njt_only);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| AppError::new(format!("schedule cache: {error}")))?;
            lock_down_dir(parent);
        }
        let bytes = serde_json::to_vec(body)
            .map_err(|error| AppError::new(format!("schedule cache: {error}")))?;
        write_private(&path, &bytes)
    }

    /// Read a named byte cache if it is still inside `max_age_secs`.
    ///
    /// A `max_age_secs` of 0 means any existing file is fresh.
    ///
    /// # Arguments
    ///
    /// * `name` - file name relative to the cache directory. Must not contain `/`.
    /// * `max_age_secs` - maximum age. Zero disables expiry.
    ///
    /// # Returns
    ///
    /// The bytes, or `None` when missing or stale.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the name is unsafe or the file cannot be read.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-bytes-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.write_bytes("example.bin", b"abc").unwrap();
    /// assert_eq!(store.read_bytes("example.bin", 0).unwrap().unwrap(), b"abc");
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn read_bytes(&self, name: &str, max_age_secs: u64) -> Result<Option<Vec<u8>>, AppError> {
        let path = self.named(name)?;
        if !path.exists() {
            return Ok(None);
        }
        if max_age_secs > 0 {
            let modified = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .map_err(|error| AppError::new(format!("cache file: {error}")))?;
            let age = modified
                .elapsed()
                .unwrap_or(std::time::Duration::from_secs(0));
            if age.as_secs() > max_age_secs {
                return Ok(None);
            }
        }
        let bytes =
            fs::read(&path).map_err(|error| AppError::new(format!("cache file: {error}")))?;
        Ok(Some(bytes))
    }

    /// Write a named byte cache as mode `0600`.
    ///
    /// # Arguments
    ///
    /// * `name` - file name without a directory separator.
    /// * `bytes` - payload.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the name is unsafe or the write fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-write-bytes-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// store.write_bytes("example.bin", b"xyz").unwrap();
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn write_bytes(&self, name: &str, bytes: &[u8]) -> Result<(), AppError> {
        let path = self.named(name)?;
        write_private(&path, bytes)
    }

    /// Counters for `njt_status`. Tokens themselves are not included.
    ///
    /// # Returns
    ///
    /// A [`Snapshot`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::store::Store;
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-store-snap-{}", std::process::id()));
    /// let store = Store::open(&dir).unwrap();
    /// assert_eq!(store.snapshot().schedule_cap, 4);
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn snapshot(&self) -> Snapshot {
        self.with_ref(|state| Snapshot {
            day: state.day.clone(),
            mints: Api::ALL.map(|api| state.mints.get(api.as_str()).copied().unwrap_or(0)),
            schedule_fetches: state.schedule_fetches,
            live_calls: state.live_calls,
            token_cached: Api::ALL.map(|api| state.tokens.contains_key(api.as_str())),
            schedule_cap: SCHEDULE_FETCH_CAP,
            live_cap: LIVE_CALL_CAP,
            rail_mint_cap: RAIL_MINT_CAP,
            bus_mint_cap: BUS_MINT_CAP,
        })
    }

    /// Path of today's schedule file for one station and NJTOnly flag.
    ///
    /// # Arguments
    ///
    /// * `station` - see the signature.
    /// * `njt_only` - see the signature.
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
    fn schedule_path(&self, station: &str, njt_only: bool) -> PathBuf {
        let day = self.with_ref(|state| state.day.clone());
        let flag = if njt_only { "njt" } else { "all" };
        self.root
            .join(format!("schedule-{station}-{flag}-{day}.json"))
    }

    /// Join a cache file name after rejecting path separators.
    ///
    /// # Arguments
    ///
    /// * `name` - see the signature.
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
    fn named(&self, name: &str) -> Result<PathBuf, AppError> {
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(AppError::new("cache file name is not allowed"));
        }
        Ok(self.root.join(name))
    }

    /// Run a closure against state, rolling the service day first.
    ///
    /// # Arguments
    ///
    /// * `body` - see the signature.
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
    fn with_ref<T>(&self, body: impl FnOnce(&State) -> T) -> T {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _ = roll_day(&mut guard);
        body(&guard)
    }

    /// Mutate state and write state.json.
    ///
    /// # Arguments
    ///
    /// * `body` - see the signature.
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
    fn with_mut(&self, body: impl FnOnce(&mut State, &Path)) -> Result<(), AppError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let path = self.root.join("state.json");
        roll_day(&mut guard)?;
        body(&mut guard, &path);
        persist(&path, &guard)
    }
}

/// Public view of counters. No token strings.
///
/// # Examples
///
/// ```
/// use njt_mcp::store::Snapshot;
/// let snapshot = Snapshot {
///     day: "2026-01-15".to_string(),
///     mints: [0, 0, 0, 0],
///     schedule_fetches: 0,
///     live_calls: 0,
///     token_cached: [false, false, false, false],
///     schedule_cap: 4,
///     live_cap: 2000,
///     rail_mint_cap: 8,
///     bus_mint_cap: 4,
/// };
/// assert_eq!(snapshot.day, "2026-01-15");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// New York service day.
    pub day: String,
    /// Mint counts in [`Api::ALL`] order.
    pub mints: [u32; 4],
    /// Schedule fetches today.
    pub schedule_fetches: u32,
    /// Live calls today.
    pub live_calls: u32,
    /// Whether each API has a cached token, in [`Api::ALL`] order.
    pub token_cached: [bool; 4],
    /// Local schedule cap.
    pub schedule_cap: u32,
    /// Local live cap.
    pub live_cap: u32,
    /// Local rail mint cap.
    pub rail_mint_cap: u32,
    /// Local bus mint cap.
    pub bus_mint_cap: u32,
}

/// Load state.json, or start empty when the file is missing or unreadable.
///
/// # Arguments
///
/// * `path` - see the signature.
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
fn load_state(path: &Path) -> (State, Option<String>) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return (State::fresh(), None),
    };
    match serde_json::from_slice::<State>(&bytes) {
        Ok(state) if state.version == 1 => (state, None),
        _ => {
            let aside = path.with_extension("json.unreadable");
            let _ = fs::rename(path, &aside);
            lock_down_file(&aside);
            (
                State::fresh(),
                Some(
                    "token cache could not be read and was reset; a previous file was kept aside without being logged"
                        .to_string(),
                ),
            )
        }
    }
}

/// Zero counters when the New York service day has changed. Tokens are kept.
///
/// # Arguments
///
/// * `state` - see the signature.
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
fn roll_day(state: &mut State) -> Result<(), AppError> {
    let today = today_service_day();
    if state.day != today {
        state.day = today;
        state.mints.clear();
        state.schedule_fetches = 0;
        state.live_calls = 0;
        state.blocked.clear();
    }
    Ok(())
}

/// Write state.json as mode 0600.
///
/// # Arguments
///
/// * `path` - see the signature.
/// * `state` - see the signature.
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
fn persist(path: &Path, state: &State) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| AppError::new(format!("token cache: {error}")))?;
    write_private(path, &bytes)
}

/// Atomically write bytes and force mode 0600.
///
/// # Arguments
///
/// * `path` - see the signature.
/// * `bytes` - see the signature.
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
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|error| AppError::new(format!("token cache: {error}")))?;
        file.write_all(bytes)
            .map_err(|error| AppError::new(format!("token cache: {error}")))?;
    }
    lock_down_file(&tmp);
    fs::rename(&tmp, path).map_err(|error| AppError::new(format!("token cache: {error}")))?;
    lock_down_file(path);
    Ok(())
}

/// Set a directory to mode 0700 on Unix.
///
/// # Arguments
///
/// * `path` - see the signature.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn lock_down_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Set a file to mode 0600 on Unix.
///
/// # Arguments
///
/// * `path` - see the signature.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn lock_down_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

impl State {
    /// Empty state for the current New York service day.
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
    fn fresh() -> Self {
        Self {
            version: 1,
            day: today_service_day(),
            tokens: BTreeMap::new(),
            mints: BTreeMap::new(),
            schedule_fetches: 0,
            live_calls: 0,
            blocked: BTreeMap::new(),
        }
    }
}

impl std::fmt::Debug for State {
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
            .debug_struct("State")
            .field("version", &self.version)
            .field("day", &self.day)
            .field("token_count", &self.tokens.len())
            .field("schedule_fetches", &self.schedule_fetches)
            .field("live_calls", &self.live_calls)
            .finish()
    }
}
