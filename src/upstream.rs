//! Classify NJ Transit HTTP bodies before any tool shapes them.
//!
//! Success payloads are returned as bytes so the caller can treat JSON, zip,
//! and protobuf differently. The documented failure shapes are handled here:
//! `errorMessage`, `Authenticated: False`, and a literal `Null` body.

use serde_json::Value;

use crate::error::{AppError, ErrorKind};

/// What an upstream body means.
///
/// # Examples
///
/// ```
/// use njt_mcp::upstream::{classify, Classified};
/// assert!(matches!(
///     classify(200, br#"{"errorMessage":"Invalid token."}"#),
///     Classified::InvalidToken
/// ));
/// assert!(matches!(classify(200, b"Null"), Classified::NullBody));
/// assert!(matches!(classify(200, b"null"), Classified::NullBody));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classified {
    /// Bytes the caller should parse as the successful payload.
    Data(Vec<u8>),
    /// `errorMessage` says the token is invalid.
    InvalidToken,
    /// `errorMessage` says a daily quota is exhausted.
    DailyLimit {
        /// Upstream text, unchanged apart from trimming.
        message: String,
    },
    /// Login JSON with `Authenticated` false.
    LoginFailed,
    /// Literal `Null` or JSON `null`.
    NullBody,
    /// HTTP 401 or 403.
    AccessDenied {
        /// Short clipped body, lossy UTF-8.
        message: String,
    },
    /// Any other `errorMessage` or non-success status.
    Message {
        /// Agent-facing detail.
        message: String,
    },
}

/// Classify a status code and body.
///
/// # Arguments
///
/// * `status` - HTTP status.
/// * `body` - raw response body. It is not logged.
///
/// # Returns
///
/// A [`Classified`] value. JSON objects without `errorMessage` and with a 2xx
/// status are [`Classified::Data`], including a successful login body.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::upstream::{classify, Classified};
/// let daily = br#"{"errorMessage":"Daily usage limit:10. Your current daily usage: 11"}"#;
/// match classify(200, daily) {
///     Classified::DailyLimit { message } => assert!(message.contains("usage: 11")),
///     other => panic!("unexpected {other:?}"),
/// }
/// assert!(matches!(
///     classify(200, br#"{"Authenticated":"False","UserToken":""}"#),
///     Classified::LoginFailed
/// ));
/// assert!(matches!(
///     classify(200, br#"{"Authenticated":"True","UserToken":"replace-me"}"#),
///     Classified::Data(_)
/// ));
/// ```
pub fn classify(status: u16, body: &[u8]) -> Classified {
    if status == 401 || status == 403 {
        return Classified::AccessDenied {
            message: clip_lossy(body, 240),
        };
    }
    let trimmed = trim_body(body);
    if trimmed.is_empty() {
        return if (200..300).contains(&status) {
            Classified::Data(body.to_vec())
        } else {
            Classified::Message {
                message: format!("HTTP {status} with an empty body"),
            }
        };
    }
    if is_null_word(trimmed) {
        return Classified::NullBody;
    }
    if let Ok(value) = serde_json::from_slice::<Value>(trimmed) {
        if value.is_null() {
            return Classified::NullBody;
        }
        if let Some(message) = object_message(&value) {
            if is_invalid_token(&message) {
                return Classified::InvalidToken;
            }
            if is_daily_limit(&message) {
                return Classified::DailyLimit { message };
            }
            return Classified::Message { message };
        }
        if authenticated_false(&value) {
            return Classified::LoginFailed;
        }
    }
    if !(200..300).contains(&status) {
        return Classified::Message {
            message: format!("HTTP {status}: {}", clip_lossy(trimmed, 240)),
        };
    }
    Classified::Data(body.to_vec())
}

/// Read a successful login body.
///
/// # Arguments
///
/// * `body` - response from `getToken` or `authenticateUser`.
///
/// # Returns
///
/// The `UserToken` string when `Authenticated` is `True`.
///
/// # Errors
///
/// Returns [`AppError`] with [`ErrorKind::LoginFailed`] when authentication
/// failed or the token is empty, [`ErrorKind::DailyLimit`] when the body is a
/// daily-limit message, and [`ErrorKind::NullBody`] for `Null`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::upstream::parse_login;
/// let token = parse_login(br#"{"Authenticated":"True","UserToken":"replace-me"}"#).unwrap();
/// assert_eq!(token, "replace-me");
/// assert!(parse_login(br#"{"Authenticated":"False","UserToken":""}"#).is_err());
/// ```
pub fn parse_login(body: &[u8]) -> Result<String, AppError> {
    match classify(200, body) {
        Classified::LoginFailed => Err(AppError::login_failed()),
        Classified::DailyLimit { message } => Err(AppError::daily_limit(
            crate::api::Api::TrainData,
            "login",
            &message,
        )),
        Classified::InvalidToken => Err(AppError::login_failed()),
        Classified::NullBody => Err(AppError::null_body("login")),
        Classified::AccessDenied { message } => Err(AppError::access_denied(
            crate::api::Api::TrainData,
            &message,
        )),
        Classified::Message { message } => Err(AppError::new(message)),
        Classified::Data(bytes) => token_from_json(&bytes),
    }
}

/// Whether `errorMessage` text is the invalid-token failure.
///
/// # Arguments
///
/// * `message` - upstream `errorMessage` value.
///
/// # Returns
///
/// `true` when the text mentions `invalid token`, ignoring ASCII case.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::upstream::is_invalid_token;
/// assert!(is_invalid_token("Invalid token."));
/// assert!(!is_invalid_token("Daily usage limit:10"));
/// ```
pub fn is_invalid_token(message: &str) -> bool {
    message.to_ascii_lowercase().contains("invalid token")
}

/// Whether `errorMessage` text is a daily quota failure.
///
/// # Arguments
///
/// * `message` - upstream `errorMessage` value.
///
/// # Returns
///
/// `true` when the text mentions a daily usage limit.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::upstream::is_daily_limit;
/// assert!(is_daily_limit(
///     "Daily usage limit:10. Your current daily usage: 11"
/// ));
/// ```
pub fn is_daily_limit(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("daily usage limit") || lower.contains("daily limit")
}

/// Map a classified failure into [`AppError`].
///
/// [`Classified::Data`] becomes a generic error because this function is only
/// for failure arms.
///
/// # Arguments
///
/// * `classified` - result of [`classify`].
/// * `api` - product that returned the body.
/// * `path` - request path, included in the message.
///
/// # Returns
///
/// An [`AppError`] whose kind matches the classification.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::upstream::{classify, into_error};
/// use njt_mcp::Api;
/// let error = into_error(classify(200, b"Null"), Api::TrainData, "/api/TrainData/getTrainSchedule");
/// assert!(error.message().contains("Null"));
/// ```
pub fn into_error(classified: Classified, api: crate::api::Api, path: &str) -> AppError {
    match classified {
        Classified::InvalidToken => AppError::new(format!("NJT rejected the token for {api}."))
            .with_kind(ErrorKind::TokenExpired),
        Classified::DailyLimit { message } => AppError::daily_limit(api, path, &message),
        Classified::LoginFailed => AppError::login_failed(),
        Classified::NullBody => AppError::null_body(path),
        Classified::AccessDenied { message } => AppError::access_denied(api, &message),
        Classified::Message { message } => AppError::new(format!("NJT {api} {path}: {message}")),
        Classified::Data(_) => {
            AppError::new(format!("NJT {api} {path} returned an unexpected body"))
        }
    }
}

/// Read `UserToken` from a login JSON object.
///
/// # Arguments
///
/// * `bytes` - body already classified as data.
///
/// # Returns
///
/// The token string.
///
/// # Errors
///
/// Returns [`AppError`] when the body is not the success object or `Authenticated` is false.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by [`parse_login`] after [`classify`].
fn token_from_json(bytes: &[u8]) -> Result<String, AppError> {
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => {
            return Err(AppError::new(
                "NJT login response was not JSON with Authenticated and UserToken.",
            ))
        }
    };
    let flag = json_text(value.get("Authenticated"));
    let token = json_text(value.get("UserToken"));
    if flag.eq_ignore_ascii_case("true") && !token.is_empty() {
        Ok(token)
    } else if flag.eq_ignore_ascii_case("false") {
        Err(AppError::login_failed())
    } else {
        Err(AppError::new(
            "NJT login response did not include a UserToken.",
        ))
    }
}

/// Pull a non-empty `errorMessage` string off a JSON value.
///
/// # Arguments
///
/// * `value` - parsed JSON.
///
/// # Returns
///
/// The message, or `None` when the field is missing or blank.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// A body `{"errorMessage":"Invalid token."}` yields `Some("Invalid token.")`.
fn object_message(value: &Value) -> Option<String> {
    let text = json_text(value.get("errorMessage"));
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Whether `Authenticated` is the string `False`.
///
/// # Arguments
///
/// * `value` - parsed JSON.
///
/// # Returns
///
/// `true` only for a false authentication flag.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// `{"Authenticated":"False"}` is `true`. `{"Authenticated":"True"}` is `false`.
fn authenticated_false(value: &Value) -> bool {
    json_text(value.get("Authenticated")).eq_ignore_ascii_case("false")
}

/// Render a JSON scalar as trimmed text.
///
/// # Arguments
///
/// * `value` - optional JSON field.
///
/// # Returns
///
/// A string, or empty when the field is missing or not a scalar.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// A JSON string `" EX "` becomes `EX` after trim. A null becomes empty.
fn json_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

/// Drop a UTF-8 BOM and surrounding ASCII whitespace.
///
/// # Arguments
///
/// * `body` - raw response bytes.
///
/// # Returns
///
/// A subslice. Empty when the body is only whitespace.
///
/// # Panics
///
/// The slice `bytes[3..]` runs only after `starts_with` confirms a 3-byte BOM.
/// The content slice runs only when `start < end`.
///
/// # Examples
///
/// `b"  Null  "` becomes `b"Null"`.
fn trim_body(body: &[u8]) -> &[u8] {
    let mut bytes = body;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes = &bytes[3..];
    }
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    if start >= end {
        &[]
    } else {
        &bytes[start..end]
    }
}

/// Whether the trimmed body is the word `null` in any ASCII case.
///
/// # Arguments
///
/// * `bytes` - trimmed body.
///
/// # Returns
///
/// `true` for `Null`, `null`, or `NULL`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// `b"Null"` is the missing-parameter body NJ Transit returns.
fn is_null_word(bytes: &[u8]) -> bool {
    bytes.eq_ignore_ascii_case(b"null")
}

/// Lossy UTF-8 clipped to `max_chars` characters.
///
/// # Arguments
///
/// * `bytes` - raw text.
/// * `max_chars` - maximum characters to keep.
///
/// # Returns
///
/// A clipped string. Invalid UTF-8 is replaced.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// A 10-byte ASCII body clipped to 4 characters yields 4 characters.
fn clip_lossy(bytes: &[u8], max_chars: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            break;
        }
        out.push(ch);
    }
    out
}
