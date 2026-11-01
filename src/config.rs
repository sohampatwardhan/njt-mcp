//! Process environment for `njt-mcp`.
//!
//! Credentials come only from the environment: `NJT_USERNAME` and
//! `NJT_PASSWORD`, or `NJT_TOKEN`, or both. Optional knobs have defaults.
//! A missing credential set or a malformed knob is a [`ConfigError`]. The
//! message names the variable and does not include secret values.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::limits::{
    DEFAULT_GTFS_TTL_SECS, DEFAULT_RT_TTL_SECS, MAX_GTFS_TTL_SECS, MAX_RT_TTL_SECS,
    MIN_RT_TTL_SECS, PROD_BUS_ORIGIN, PROD_RAIL_ORIGIN, TEST_BUS_ORIGIN, TEST_RAIL_ORIGIN,
};

/// How the process will authenticate.
///
/// # Examples
///
/// ```
/// use njt_mcp::{Config, CredentialMode};
/// let config = Config::parse([
///     ("NJT_TOKEN", "replace-me"),
///     ("NJT_USERNAME", "replace-me"),
///     ("NJT_PASSWORD", "replace-me"),
///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
/// ])
/// .unwrap();
/// assert_eq!(config.credential_mode(), CredentialMode::TokenAndPassword);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialMode {
    /// `NJT_TOKEN` is set and username/password are not.
    TokenOnly,
    /// `NJT_USERNAME` and `NJT_PASSWORD` are set and `NJT_TOKEN` is not.
    Password,
    /// Both forms are set. The token is tried first.
    TokenAndPassword,
}

/// Which NJ Transit host pair to use when origins are not overridden.
///
/// # Examples
///
/// ```
/// use njt_mcp::{Config, Environment};
/// let config = Config::parse([
///     ("NJT_USERNAME", "replace-me"),
///     ("NJT_PASSWORD", "replace-me"),
///     ("NJT_ENV", "test"),
///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
/// ])
/// .unwrap();
/// assert_eq!(config.environment(), Environment::Test);
/// assert_eq!(config.rail_origin(), "https://testraildata.njtransit.com");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// `raildata.njtransit.com` and `pcsdata.njtransit.com`.
    Production,
    /// `testraildata.njtransit.com` and `testpcsdata.njtransit.com`.
    Test,
}

/// Startup failure. Display text is safe to print.
///
/// # Examples
///
/// ```
/// use njt_mcp::Config;
/// let error = Config::parse([("NJT_TOKEN", "super-secret-token"), ("NJT_ENV", "nope")]).unwrap_err();
/// let text = error.to_string();
/// assert!(text.contains("NJT_ENV"));
/// assert!(!text.contains("super-secret-token"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// Joined problems. Secret values are not copied here.
    message: String,
}

impl ConfigError {
    /// Borrow the message.
    ///
    /// # Returns
    ///
    /// The full configuration error.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let error = Config::parse(Vec::<(&str, &str)>::new()).unwrap_err();
    /// assert!(error.message().contains("NJT_TOKEN"));
    /// assert!(error.message().contains("NJT_USERNAME"));
    /// ```
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ConfigError {
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
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Credentials held in memory. `Debug` does not print them.
#[derive(Clone)]
pub struct Credentials {
    /// Pre-minted token from `NJT_TOKEN`, when set.
    token: Option<String>,
    /// Portal username, when set.
    username: Option<String>,
    /// Portal password, when set.
    password: Option<String>,
}

impl Credentials {
    /// Which form was configured.
    ///
    /// # Returns
    ///
    /// The [`CredentialMode`]. A value of this type always has at least one form,
    /// so this does not fail.
    ///
    /// # Panics
    ///
    /// This function does not panic. It treats a structurally empty value as
    /// [`CredentialMode::Password`] only when no token is present; [`Config::parse`]
    /// never builds that state.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Config, CredentialMode};
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(
    ///     config.credentials().mode(),
    ///     CredentialMode::TokenOnly
    /// );
    /// ```
    pub fn mode(&self) -> CredentialMode {
        match (self.token.is_some(), self.username.is_some()) {
            (true, true) => CredentialMode::TokenAndPassword,
            (true, false) => CredentialMode::TokenOnly,
            (false, _) => CredentialMode::Password,
        }
    }

    /// Whether a password pair is available for minting.
    ///
    /// # Returns
    ///
    /// `true` when both username and password are set.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert!(!config.credentials().has_password());
    /// ```
    pub fn has_password(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }

    /// Borrow the pre-minted token.
    ///
    /// # Returns
    ///
    /// The token, or `None` in password-only mode.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.credentials().token(), Some("replace-me"));
    /// ```
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Borrow the username.
    ///
    /// # Returns
    ///
    /// The username, or `None` in token-only mode.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_USERNAME", "replace-me"),
    ///     ("NJT_PASSWORD", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.credentials().username(), Some("replace-me"));
    /// ```
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// Borrow the password.
    ///
    /// # Returns
    ///
    /// The password, or `None` in token-only mode.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_USERNAME", "replace-me"),
    ///     ("NJT_PASSWORD", "replace-me-password"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.credentials().password(), Some("replace-me-password"));
    /// ```
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// Secret strings that must be stripped from errors and logs.
    ///
    /// # Returns
    ///
    /// Username, password, and token values that are present.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.credentials().secrets(), vec!["replace-me".to_string()]);
    /// ```
    pub fn secrets(&self) -> Vec<String> {
        let mut values = Vec::new();
        if let Some(token) = &self.token {
            values.push(token.clone());
        }
        if let Some(username) = &self.username {
            values.push(username.clone());
        }
        if let Some(password) = &self.password {
            values.push(password.clone());
        }
        values
    }
}

impl std::fmt::Debug for Credentials {
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
            .debug_struct("Credentials")
            .field("mode", &self.mode())
            .finish()
    }
}

/// Parsed startup configuration.
///
/// # Examples
///
/// ```
/// use njt_mcp::Config;
/// let config = Config::from_env();
/// let _ = config;
/// ```
///
/// The snippet above reads the real environment. Library tests should call
/// [`Config::parse`] with explicit pairs instead.
#[derive(Clone)]
pub struct Config {
    /// Credentials. Not printed by `Debug`.
    credentials: Credentials,
    /// Production or test, before origin overrides.
    environment: Environment,
    /// Rail host origin without a trailing slash.
    rail_origin: String,
    /// Bus host origin without a trailing slash.
    bus_origin: String,
    /// Directory for the token file, schedules, and GTFS zips.
    cache_dir: PathBuf,
    /// Live response cache TTL in seconds.
    rt_ttl_secs: u64,
    /// GTFS zip TTL in seconds. `0` keeps the first downloaded zip.
    gtfs_ttl_secs: u64,
}

impl Config {
    /// Parse the process environment.
    ///
    /// # Returns
    ///
    /// A [`Config`] when the environment is valid.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] for the same reasons as [`Config::parse`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use njt_mcp::Config;
    /// match Config::from_env() {
    ///     Ok(config) => println!("{}", config.credential_mode().as_str()),
    ///     Err(error) => eprintln!("njt-mcp: {error}"),
    /// }
    /// ```
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::parse(std::env::vars())
    }

    /// Parse an explicit set of variables.
    ///
    /// Empty values are ignored. Later duplicates override earlier ones.
    ///
    /// # Arguments
    ///
    /// * `vars` - key/value pairs. Keys are matched exactly.
    ///
    /// # Returns
    ///
    /// A [`Config`].
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when neither credential form is complete, when
    /// only one of `NJT_USERNAME` and `NJT_PASSWORD` is set, or when an optional
    /// knob is malformed. The message never includes credential values.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Config, Environment};
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_ENV", "production"),
    ///     ("NJT_RT_TTL", "15"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.environment(), Environment::Production);
    /// assert_eq!(config.rt_ttl_secs(), 15);
    /// assert_eq!(config.rail_origin(), "https://raildata.njtransit.com");
    /// ```
    pub fn parse(
        vars: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
    ) -> Result<Self, ConfigError> {
        let vars = collect_vars(vars);
        let mut problems = Vec::new();
        let token = vars.get("NJT_TOKEN").cloned();
        let username = vars.get("NJT_USERNAME").cloned();
        let password = vars.get("NJT_PASSWORD").cloned();
        push_credential_problems(
            &mut problems,
            token.as_deref(),
            username.as_deref(),
            password.as_deref(),
        );

        let environment = match vars.get("NJT_ENV").map(String::as_str) {
            None => Environment::Production,
            Some("production") => Environment::Production,
            Some("test") => Environment::Test,
            Some(other) => {
                if secret_like(
                    other,
                    token.as_deref(),
                    username.as_deref(),
                    password.as_deref(),
                ) {
                    problems.push("NJT_ENV must be production or test".to_string());
                } else {
                    problems.push(format!("NJT_ENV must be production or test, got {other}"));
                }
                Environment::Production
            }
        };

        let rt_ttl_secs = match optional_u64(
            vars.get("NJT_RT_TTL"),
            "NJT_RT_TTL",
            MIN_RT_TTL_SECS,
            MAX_RT_TTL_SECS,
            &token,
            &username,
            &password,
        ) {
            Ok(value) => value.unwrap_or(DEFAULT_RT_TTL_SECS),
            Err(problem) => {
                problems.push(problem);
                DEFAULT_RT_TTL_SECS
            }
        };
        let gtfs_ttl_secs = match optional_u64(
            vars.get("NJT_GTFS_TTL"),
            "NJT_GTFS_TTL",
            0,
            MAX_GTFS_TTL_SECS,
            &token,
            &username,
            &password,
        ) {
            Ok(value) => value.unwrap_or(DEFAULT_GTFS_TTL_SECS),
            Err(problem) => {
                problems.push(problem);
                DEFAULT_GTFS_TTL_SECS
            }
        };

        let (default_rail, default_bus) = match environment {
            Environment::Production => (PROD_RAIL_ORIGIN, PROD_BUS_ORIGIN),
            Environment::Test => (TEST_RAIL_ORIGIN, TEST_BUS_ORIGIN),
        };
        let rail_origin =
            match origin_of(vars.get("NJT_RAIL_ORIGIN"), "NJT_RAIL_ORIGIN", default_rail) {
                Ok(origin) => origin,
                Err(problem) => {
                    problems.push(problem);
                    default_rail.to_string()
                }
            };
        let bus_origin = match origin_of(vars.get("NJT_BUS_ORIGIN"), "NJT_BUS_ORIGIN", default_bus)
        {
            Ok(origin) => origin,
            Err(problem) => {
                problems.push(problem);
                default_bus.to_string()
            }
        };
        let cache_dir = match cache_dir_of(vars.get("NJT_CACHE_DIR")) {
            Ok(path) => path,
            Err(problem) => {
                problems.push(problem);
                PathBuf::from("/tmp/njt-mcp-unconfigured")
            }
        };

        if !problems.is_empty() {
            return Err(ConfigError {
                message: problems.join("; "),
            });
        }
        Ok(Self {
            credentials: Credentials {
                token,
                username,
                password,
            },
            environment,
            rail_origin,
            bus_origin,
            cache_dir,
            rt_ttl_secs,
            gtfs_ttl_secs,
        })
    }

    /// Borrow the credentials.
    ///
    /// # Returns
    ///
    /// The parsed [`Credentials`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert!(config.credentials().token().is_some());
    /// ```
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// Credential form, without revealing the values.
    ///
    /// # Returns
    ///
    /// The [`CredentialMode`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Config, CredentialMode};
    /// let config = Config::parse([
    ///     ("NJT_USERNAME", "replace-me"),
    ///     ("NJT_PASSWORD", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.credential_mode(), CredentialMode::Password);
    /// ```
    pub fn credential_mode(&self) -> CredentialMode {
        self.credentials.mode()
    }

    /// Selected environment.
    ///
    /// # Returns
    ///
    /// [`Environment::Production`] or [`Environment::Test`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Config, Environment};
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.environment(), Environment::Production);
    /// ```
    pub fn environment(&self) -> Environment {
        self.environment
    }

    /// Rail origin, without a trailing slash.
    ///
    /// # Returns
    ///
    /// An `http` or `https` origin.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_RAIL_ORIGIN", "http://127.0.0.1:9"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.rail_origin(), "http://127.0.0.1:9");
    /// ```
    pub fn rail_origin(&self) -> &str {
        &self.rail_origin
    }

    /// Bus origin, without a trailing slash.
    ///
    /// # Returns
    ///
    /// An `http` or `https` origin.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_BUS_ORIGIN", "http://127.0.0.1:9"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.bus_origin(), "http://127.0.0.1:9");
    /// ```
    pub fn bus_origin(&self) -> &str {
        &self.bus_origin
    }

    /// Cache directory. It is created when the service starts, not during parse.
    ///
    /// # Returns
    ///
    /// The configured path.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.cache_dir().as_os_str(), "/tmp/njt-mcp-doctest");
    /// ```
    pub fn cache_dir(&self) -> &std::path::Path {
        &self.cache_dir
    }

    /// Live cache TTL.
    ///
    /// # Returns
    ///
    /// Seconds, from 5 to 300.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.rt_ttl_secs(), 30);
    /// ```
    pub fn rt_ttl_secs(&self) -> u64 {
        self.rt_ttl_secs
    }

    /// Live cache TTL as a [`Duration`].
    ///
    /// # Returns
    ///
    /// The same length as [`Config::rt_ttl_secs`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.rt_ttl().as_secs(), 30);
    /// ```
    pub fn rt_ttl(&self) -> Duration {
        Duration::from_secs(self.rt_ttl_secs)
    }

    /// GTFS zip TTL. Zero means a downloaded zip is reused until it is deleted.
    ///
    /// # Returns
    ///
    /// Seconds, from 0 to 604800.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_GTFS_TTL", "0"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert_eq!(config.gtfs_ttl_secs(), 0);
    /// ```
    pub fn gtfs_ttl_secs(&self) -> u64 {
        self.gtfs_ttl_secs
    }
}

impl std::fmt::Debug for Config {
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
            .debug_struct("Config")
            .field("credential_mode", &self.credential_mode())
            .field("environment", &self.environment)
            .field("rail_origin", &self.rail_origin)
            .field("bus_origin", &self.bus_origin)
            .field("cache_dir", &self.cache_dir)
            .field("rt_ttl_secs", &self.rt_ttl_secs)
            .field("gtfs_ttl_secs", &self.gtfs_ttl_secs)
            .finish()
    }
}

impl CredentialMode {
    /// Stable status-JSON value.
    ///
    /// # Returns
    ///
    /// `token`, `password`, or `token_and_password`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::CredentialMode;
    /// assert_eq!(CredentialMode::TokenOnly.as_str(), "token");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TokenOnly => "token",
            Self::Password => "password",
            Self::TokenAndPassword => "token_and_password",
        }
    }
}

impl std::fmt::Display for CredentialMode {
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
        formatter.write_str(self.as_str())
    }
}

impl std::fmt::Display for Environment {
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
        formatter.write_str(match self {
            Self::Production => "production",
            Self::Test => "test",
        })
    }
}

/// Trim values, drop empties, and keep the last duplicate key.
///
/// # Arguments
///
/// * `vars` - raw pairs. Keys are matched exactly.
///
/// # Returns
///
/// A map of non-empty values.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// `("NJT_TOKEN", "  ")` is dropped. A later `NJT_TOKEN` replaces an earlier one.
fn collect_vars(
    vars: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (key, value) in vars {
        let value = value.as_ref().trim();
        if value.is_empty() {
            continue;
        }
        out.insert(key.as_ref().to_string(), value.to_string());
    }
    out
}

/// Record missing or half-specified credentials.
///
/// # Arguments
///
/// * `problems` - messages collected for [`ConfigError`].
/// * `token` - `NJT_TOKEN`, if set.
/// * `username` - `NJT_USERNAME`, if set.
/// * `password` - `NJT_PASSWORD`, if set.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Username without password appends a message that names `NJT_PASSWORD` and not the username.
fn push_credential_problems(
    problems: &mut Vec<String>,
    token: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
) {
    match (username, password) {
        (Some(_), None) => {
            problems.push(
                "NJT_PASSWORD is required when NJT_USERNAME is set. Set both, or set NJT_TOKEN instead."
                    .to_string(),
            );
        }
        (None, Some(_)) => {
            problems.push(
                "NJT_USERNAME is required when NJT_PASSWORD is set. Set both, or set NJT_TOKEN instead."
                    .to_string(),
            );
        }
        (None, None) if token.is_none() => {
            problems.push(
                "NJT_TOKEN is required, or set both NJT_USERNAME and NJT_PASSWORD. Example: NJT_TOKEN=replace-me"
                    .to_string(),
            );
        }
        _ => {}
    }
}

/// Whether a malformed value is actually a credential, so it must not be echoed.
///
/// # Arguments
///
/// * `value` - the candidate text.
/// * `token` - `NJT_TOKEN`, if set.
/// * `username` - `NJT_USERNAME`, if set.
/// * `password` - `NJT_PASSWORD`, if set.
///
/// # Returns
///
/// `true` when `value` equals one of the secrets.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// If `NJT_ENV` is set to the same string as `NJT_TOKEN`, the error omits that string.
fn secret_like(
    value: &str,
    token: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
) -> bool {
    [token, username, password]
        .into_iter()
        .flatten()
        .any(|secret| !secret.is_empty() && secret == value)
}

/// Parse an optional integer TTL.
///
/// # Arguments
///
/// * `raw` - variable value.
/// * `name` - variable name used in errors.
/// * `min` - inclusive minimum.
/// * `max` - inclusive maximum.
/// * `token` - credential used only to suppress echoing.
/// * `username` - credential used only to suppress echoing.
/// * `password` - credential used only to suppress echoing.
///
/// # Returns
///
/// `None` when the variable is unset, or the parsed integer.
///
/// # Errors
///
/// Returns a string error when the value is not an integer or is outside the range.
/// The error does not include a value that matches a credential.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// `NJT_RT_TTL=15` with min 5 and max 300 returns `Some(15)`.
fn optional_u64(
    raw: Option<&String>,
    name: &str,
    min: u64,
    max: u64,
    token: &Option<String>,
    username: &Option<String>,
    password: &Option<String>,
) -> Result<Option<u64>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if secret_like(
        raw,
        token.as_deref(),
        username.as_deref(),
        password.as_deref(),
    ) {
        return Err(format!("{name} is not an integer number of seconds"));
    }
    let value = match raw.parse::<u64>() {
        Ok(value) => value,
        Err(_) => {
            return Err(format!("{name}: {raw} is not an integer number of seconds"));
        }
    };
    if value < min || value > max {
        return Err(format!(
            "{name} must be an integer from {min} to {max} seconds, got {value}"
        ));
    }
    Ok(Some(value))
}

/// Validate an optional origin override.
///
/// # Arguments
///
/// * `raw` - variable value.
/// * `name` - variable name used in errors.
/// * `default` - origin used when the variable is unset.
///
/// # Returns
///
/// `scheme://host` with an explicit port when one was present.
///
/// # Errors
///
/// Returns a string error when the value is not an http(s) origin. The raw
/// value is not echoed, because it might contain userinfo.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// `http://127.0.0.1:9` is accepted. `http://user:secret@example.com` is rejected
/// without copying `secret` into the error.
fn origin_of(raw: Option<&String>, name: &str, default: &str) -> Result<String, String> {
    let Some(raw) = raw else {
        return Ok(default.to_string());
    };
    let trimmed = raw.trim().trim_end_matches('/');
    let url = match reqwest::Url::parse(trimmed) {
        Ok(url) => url,
        Err(_) => {
            return Err(format!(
                "{name} must be an http(s) origin with no userinfo, path, query, or fragment"
            ));
        }
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(format!(
            "{name} must be an http(s) origin with no userinfo, path, query, or fragment"
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{name} must not include a username or password"));
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err(format!("{name} must be an origin without a path"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!(
            "{name} must be an origin without a query or fragment"
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| format!("{name} is missing a host"))?;
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Ok(format!("{}://{host}{port}", url.scheme()))
}

/// Resolve the cache directory.
///
/// # Arguments
///
/// * `raw` - `NJT_CACHE_DIR`, when set.
///
/// # Returns
///
/// The explicit path, or `$XDG_CACHE_HOME/njt-mcp`, or `$HOME/.cache/njt-mcp`.
///
/// # Errors
///
/// Returns a string error when no directory can be chosen.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// An explicit `/tmp/njt-mcp-doctest` is returned unchanged and is not created.
fn cache_dir_of(raw: Option<&String>) -> Result<PathBuf, String> {
    if let Some(raw) = raw {
        return Ok(PathBuf::from(raw));
    }
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("njt-mcp"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home).join(".cache").join("njt-mcp"));
        }
    }
    Err("NJT_CACHE_DIR is required when HOME and XDG_CACHE_HOME are unset".to_string())
}
