//! Errors an agent should read.
//!
//! Messages name the NJ Transit product and the next step. They never include
//! usernames, passwords, or tokens. [`redact`] is the last filter applied
//! before a message is stored or returned.

use serde_json::{Map, Value};

use crate::api::Api;

/// Why a call failed, for metrics and tests.
///
/// Agents should read [`AppError::message`], not match on this enum, so new
/// cases can be added without breaking tool output.
///
/// # Examples
///
/// ```
/// use njt_mcp::{Api, AppError, ErrorKind};
/// let error = AppError::token_expired(Api::TrainData);
/// assert_eq!(error.kind(), ErrorKind::TokenExpired);
/// assert!(error.message().contains("expired"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Anything that is not one of the more specific cases.
    Other,
    /// Username and password were rejected.
    LoginFailed,
    /// `NJT_TOKEN` was rejected and this process cannot mint another.
    TokenExpired,
    /// NJ Transit returned a daily-usage error for one path.
    DailyLimit,
    /// A local cap stopped the call before it reached NJ Transit.
    LocalCap,
    /// HTTP 401 or 403, or an access-denied body.
    AccessDenied,
    /// The literal `Null` body NJ Transit uses for bad parameters.
    NullBody,
}

/// Failure returned to an MCP client as compact JSON.
///
/// # Examples
///
/// ```
/// use njt_mcp::AppError;
/// let error = AppError::new("station is empty").hint("Call rail_stations first.");
/// let json = error.to_json();
/// assert_eq!(json["error"], "station is empty");
/// assert_eq!(json["hint"], "Call rail_stations first.");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    /// Text placed in the `error` field. Must already be free of secrets.
    message: String,
    /// Optional `hint` field.
    hint: Option<String>,
    /// Classifier for tests.
    kind: ErrorKind,
}

impl AppError {
    /// Build an error with [`ErrorKind::Other`] and no hint.
    ///
    /// # Arguments
    ///
    /// * `message` - agent-facing text. The caller must already have removed secrets.
    ///
    /// # Returns
    ///
    /// An [`AppError`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{AppError, ErrorKind};
    /// let error = AppError::new("station is empty");
    /// assert_eq!(error.kind(), ErrorKind::Other);
    /// assert!(error.hint_text().is_none());
    /// ```
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            hint: None,
            kind: ErrorKind::Other,
        }
    }

    /// Attach a hint.
    ///
    /// # Arguments
    ///
    /// * `hint` - short next step. Do not put credentials in it.
    ///
    /// # Returns
    ///
    /// The same error with the hint set.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::AppError;
    /// let error = AppError::new("cache directory").hint("Set NJT_CACHE_DIR.");
    /// assert_eq!(error.hint_text(), Some("Set NJT_CACHE_DIR."));
    /// ```
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Replace the kind. Used by constructors in this module.
    ///
    /// # Arguments
    ///
    /// * `kind` - classifier stored alongside the message.
    ///
    /// # Returns
    ///
    /// The same error with the new kind.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{AppError, ErrorKind};
    /// let error = AppError::new("nope").with_kind(ErrorKind::NullBody);
    /// assert_eq!(error.kind(), ErrorKind::NullBody);
    /// ```
    pub fn with_kind(mut self, kind: ErrorKind) -> Self {
        self.kind = kind;
        self
    }

    /// Message text.
    ///
    /// # Returns
    ///
    /// The `error` string.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::AppError;
    /// assert_eq!(AppError::new("x").message(), "x");
    /// ```
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Hint text, when one was set.
    ///
    /// # Returns
    ///
    /// The hint, or `None`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::AppError;
    /// assert_eq!(AppError::new("x").hint("y").hint_text(), Some("y"));
    /// ```
    pub fn hint_text(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// Kind stored on the error.
    ///
    /// # Returns
    ///
    /// The [`ErrorKind`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Api, AppError, ErrorKind};
    /// assert_eq!(
    ///     AppError::token_expired(Api::GtfsRt).kind(),
    ///     ErrorKind::TokenExpired
    /// );
    /// ```
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// JSON object with `error` and, when present, `hint`.
    ///
    /// # Returns
    ///
    /// A JSON object. Serialization of these two strings cannot fail in practice;
    /// a failure becomes `{"error":"failed to serialize"}`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::AppError;
    /// let value = AppError::new("station is empty").to_json();
    /// assert!(value.get("hint").is_none());
    /// ```
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("error".to_string(), Value::String(self.message.clone()));
        if let Some(hint) = &self.hint {
            map.insert("hint".to_string(), Value::String(hint.clone()));
        }
        Value::Object(map)
    }

    /// Token-only mode hit an invalid-token response.
    ///
    /// # Arguments
    ///
    /// * `api` - product that rejected the token.
    ///
    /// # Returns
    ///
    /// An error whose message says the token expired and names `NJT_TOKEN`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Api, AppError};
    /// let error = AppError::token_expired(Api::BusDv2);
    /// assert!(error.message().contains("expired"));
    /// assert!(error.message().contains("NJT_TOKEN"));
    /// assert!(error.message().contains("BUSDV2"));
    /// ```
    pub fn token_expired(api: Api) -> Self {
        Self::new(format!(
            "NJT token expired for {api}. This process was started with NJT_TOKEN only and cannot mint a replacement. Set NJT_USERNAME and NJT_PASSWORD, or restart with a fresh NJT_TOKEN."
        ))
        .hint("One NJT_TOKEN is not guaranteed to be accepted by every API. Tokens are minted per product.")
        .with_kind(ErrorKind::TokenExpired)
    }

    /// Username and password were rejected, or the login body had no token.
    ///
    /// # Returns
    ///
    /// A login-failed error. The message does not include the username.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{AppError, ErrorKind};
    /// assert_eq!(AppError::login_failed().kind(), ErrorKind::LoginFailed);
    /// ```
    pub fn login_failed() -> Self {
        Self::new(
            "NJT rejected the username and password (Authenticated is False). Check NJT_USERNAME and NJT_PASSWORD, and request access to this API on the developer portal.",
        )
        .with_kind(ErrorKind::LoginFailed)
    }

    /// Local mint cap reached.
    ///
    /// # Arguments
    ///
    /// * `api` - product whose counter is exhausted.
    ///
    /// # Returns
    ///
    /// A [`ErrorKind::LocalCap`] error.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Api, AppError};
    /// assert!(AppError::mint_cap(Api::TrainData).message().contains("8"));
    /// ```
    pub fn mint_cap(api: Api) -> Self {
        Self::new(format!(
            "Local login cap reached for {api} ({cap} per New York day). This server stays under NJ Transit's getToken budget and will not call login again until the next New York midnight.",
            cap = api.mint_cap()
        ))
        .with_kind(ErrorKind::LocalCap)
    }

    /// Local `getStationSchedule` cap reached.
    ///
    /// # Returns
    ///
    /// A [`ErrorKind::LocalCap`] error that mentions the cap of 4.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::AppError;
    /// assert!(AppError::schedule_cap().message().contains("getStationSchedule"));
    /// ```
    pub fn schedule_cap() -> Self {
        Self::new(
            "Local cap reached for getStationSchedule (4 per New York day, under NJ Transit's limit of 5). Schedules already fetched today are still served from disk.",
        )
        .with_kind(ErrorKind::LocalCap)
    }

    /// Local live-call cap reached.
    ///
    /// # Returns
    ///
    /// A [`ErrorKind::LocalCap`] error.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::AppError;
    /// assert!(AppError::live_cap().message().contains("40,000"));
    /// ```
    pub fn live_cap() -> Self {
        Self::new(
            "Local cap reached for live NJ Transit calls (2000 per New York day, under the 40,000 current-data budget). Ask again tomorrow, or use data already returned in this session.",
        )
        .hint("Do not poll vehicle or departure tools. Call them when a person asks.")
        .with_kind(ErrorKind::LocalCap)
    }

    /// Upstream daily-limit message.
    ///
    /// # Arguments
    ///
    /// * `api` - product that returned the message.
    /// * `path` - request path, used only as a label.
    /// * `upstream` - `errorMessage` text from NJ Transit.
    ///
    /// # Returns
    ///
    /// A [`ErrorKind::DailyLimit`] error containing the upstream text.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Api, AppError, ErrorKind};
    /// let error = AppError::daily_limit(
    ///     Api::TrainData,
    ///     "/api/TrainData/getToken",
    ///     "Daily usage limit:10. Your current daily usage: 11",
    /// );
    /// assert_eq!(error.kind(), ErrorKind::DailyLimit);
    /// assert!(error.message().contains("usage: 11"));
    /// ```
    pub fn daily_limit(api: Api, path: &str, upstream: &str) -> Self {
        Self::new(format!(
            "NJT daily limit reached for {api} {path}: {upstream}"
        ))
        .hint("This server will not retry this path until the next New York midnight.")
        .with_kind(ErrorKind::DailyLimit)
    }

    /// Literal `Null` body.
    ///
    /// # Arguments
    ///
    /// * `path` - request path that returned `Null`.
    ///
    /// # Returns
    ///
    /// A [`ErrorKind::NullBody`] error.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::AppError;
    /// assert!(AppError::null_body("/api/TrainData/getTrainSchedule")
    ///     .message()
    ///     .contains("Null"));
    /// ```
    pub fn null_body(path: &str) -> Self {
        Self::new(format!(
            "NJT returned Null for {path}, which means a required parameter was missing or invalid."
        ))
        .with_kind(ErrorKind::NullBody)
    }

    /// The account cannot call this product.
    ///
    /// # Arguments
    ///
    /// * `api` - product that refused the call.
    /// * `detail` - short redacted detail, may be empty.
    ///
    /// # Returns
    ///
    /// A [`ErrorKind::AccessDenied`] error.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Api, AppError};
    /// let error = AppError::access_denied(Api::GtfsG2, "HTTP 403");
    /// assert!(error.message().contains("GTFSG2"));
    /// ```
    pub fn access_denied(api: Api, detail: &str) -> Self {
        let detail = detail.trim();
        let message = if detail.is_empty() {
            format!(
                "NJT denied access to {api}. Request access to this API for the developer-portal app that issued these credentials."
            )
        } else {
            format!(
                "NJT denied access to {api}. Request access to this API for the developer-portal app that issued these credentials. {detail}"
            )
        };
        Self::new(message).with_kind(ErrorKind::AccessDenied)
    }
}

impl std::fmt::Display for AppError {
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

impl std::error::Error for AppError {}

/// Replace known secret strings with `[redacted]`.
///
/// Secrets shorter than 4 characters are left in place so a value like `a`
/// cannot blank out ordinary prose. The function is pure.
///
/// # Arguments
///
/// * `text` - message that might have picked up a credential.
/// * `secrets` - credential values. Empty strings are ignored.
///
/// # Returns
///
/// `text` with each qualifying secret replaced.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::redact;
/// let cleaned = redact(
///     "token super-secret-token failed",
///     &["super-secret-token"],
/// );
/// assert!(!cleaned.contains("super-secret-token"));
/// assert!(cleaned.contains("[redacted]"));
/// assert_eq!(redact("abc", &["ab"]), "abc");
/// ```
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut cleaned = text.to_string();
    for secret in secrets {
        if secret.len() >= 4 {
            cleaned = cleaned.replace(secret, "[redacted]");
        }
    }
    cleaned
}

/// Copy an error after running [`redact`] on the message and hint.
///
/// # Arguments
///
/// * `error` - original error.
/// * `secrets` - values that must not appear in tool output.
///
/// # Returns
///
/// An error with the same [`ErrorKind`] and scrubbed text.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::{redact_error, AppError};
/// let error = redact_error(AppError::new("saw super-secret-token"), &["super-secret-token"]);
/// assert!(!error.message().contains("super-secret-token"));
/// ```
pub fn redact_error(error: AppError, secrets: &[&str]) -> AppError {
    let hint = error.hint.clone().map(|value| redact(&value, secrets));
    let mut cleaned = AppError::new(redact(&error.message, secrets)).with_kind(error.kind);
    if let Some(hint) = hint {
        cleaned = cleaned.hint(hint);
    }
    cleaned
}
