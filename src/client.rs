//! HTTP POST multipart client for the NJ Transit form APIs.
//!
//! The token is a form field named `token`. It is never written to the URL and
//! never written to tracing fields.

use std::time::Duration;

use futures_util::StreamExt;

use crate::api::{Api, OriginHost};
use crate::config::Config;
use crate::error::AppError;
use crate::limits::{JSON_MAX_BYTES, PROTO_MAX_BYTES, ZIP_MAX_BYTES};

/// One HTTP response, status plus a size-capped body.
///
/// # Examples
///
/// ```
/// use njt_mcp::client::RawResponse;
/// let response = RawResponse { status: 200, bytes: b"[]".to_vec() };
/// assert!(response.is_success());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawResponse {
    /// HTTP status code.
    pub status: u16,
    /// Body, possibly truncated only by the caller's maximum.
    pub bytes: Vec<u8>,
}

impl RawResponse {
    /// Whether the status is in the 2xx range.
    ///
    /// # Returns
    ///
    /// `true` for 200 through 299.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::client::RawResponse;
    /// let response = RawResponse { status: 403, bytes: Vec::new() };
    /// assert!(!response.is_success());
    /// ```
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Shared `reqwest` client plus the two configured origins.
///
/// # Examples
///
/// ```no_run
/// use njt_mcp::{Api, Config};
/// use njt_mcp::client::Client;
/// # fn demo() -> Result<(), njt_mcp::ConfigError> {
/// let config = Config::parse([
///     ("NJT_TOKEN", "replace-me"),
///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
/// ])?;
/// let client = Client::new(&config).expect("client");
/// assert!(client.url(Api::TrainData, "/api/TrainData/getStationList").contains("raildata"));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Client {
    /// HTTP implementation. Debug does not expand the connection pool.
    http: reqwest::Client,
    /// Rail origin, no trailing slash.
    rail_origin: String,
    /// Bus origin, no trailing slash.
    bus_origin: String,
}

impl Client {
    /// Build a client from parsed configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - origins and nothing else. Credentials are not stored here.
    ///
    /// # Returns
    ///
    /// A [`Client`] with a 10 second connect timeout and a 45 second overall timeout.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the TLS client cannot be built.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::client::Client;
    /// use njt_mcp::Config;
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// assert!(Client::new(&config).is_ok());
    /// ```
    pub fn new(config: &Config) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .user_agent(concat!("njt-mcp/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(|error| AppError::new(format!("http client: {error}")))?;
        Ok(Self {
            http,
            rail_origin: config.rail_origin().to_string(),
            bus_origin: config.bus_origin().to_string(),
        })
    }

    /// Absolute URL for one path on the product's origin.
    ///
    /// # Arguments
    ///
    /// * `api` - selects the rail or bus origin.
    /// * `path` - absolute path beginning with `/`.
    ///
    /// # Returns
    ///
    /// `origin + path`. The token is not included.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::client::Client;
    /// use njt_mcp::{Api, Config};
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_ENV", "test"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// let client = Client::new(&config).unwrap();
    /// assert_eq!(
    ///     client.url(Api::BusDv2, "/api/BUSDV2/getLocations"),
    ///     "https://testpcsdata.njtransit.com/api/BUSDV2/getLocations"
    /// );
    /// ```
    pub fn url(&self, api: Api, path: &str) -> String {
        let origin = match api.origin_host() {
            OriginHost::Rail => self.rail_origin.as_str(),
            OriginHost::Bus => self.bus_origin.as_str(),
        };
        format!("{origin}{path}")
    }

    /// POST `multipart/form-data` and read the body up to `max_bytes`.
    ///
    /// # Arguments
    ///
    /// * `url` - absolute URL. Must not contain credentials.
    /// * `fields` - form fields. Values are not logged.
    /// * `max_bytes` - hard cap. Larger bodies return an error and are not kept.
    ///
    /// # Returns
    ///
    /// Status and body.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] on transport failure or when the body exceeds `max_bytes`.
    /// The transport error does not include form values.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use njt_mcp::client::Client;
    /// use njt_mcp::limits::JSON_MAX_BYTES;
    /// use njt_mcp::Config;
    /// # async fn demo() {
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
    /// ])
    /// .unwrap();
    /// let client = Client::new(&config).unwrap();
    /// let _ = client
    ///     .post_form(
    ///         "https://example.com/api/TrainData/getStationList",
    ///         &[("token", "replace-me")],
    ///         JSON_MAX_BYTES,
    ///     )
    ///     .await;
    /// # }
    /// ```
    pub async fn post_form(
        &self,
        url: &str,
        fields: &[(&str, &str)],
        max_bytes: usize,
    ) -> Result<RawResponse, AppError> {
        let mut form = reqwest::multipart::Form::new();
        for (name, value) in fields {
            form = form.text((*name).to_string(), (*value).to_string());
        }
        let response = self
            .http
            .post(url)
            .multipart(form)
            .send()
            .await
            .map_err(|error| AppError::new(format!("http: {error}")))?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| AppError::new(format!("http: {error}")))?;
            let next = bytes.len().saturating_add(chunk.len());
            if next > max_bytes {
                return Err(AppError::new(format!(
                    "NJT response exceeded {max_bytes} bytes"
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(RawResponse { status, bytes })
    }
}

/// Pick a body cap for a path.
///
/// # Arguments
///
/// * `path` - absolute API path.
///
/// # Returns
///
/// [`ZIP_MAX_BYTES`] for `getGTFS`, [`PROTO_MAX_BYTES`] for GTFS-Realtime
/// protobuf methods, otherwise [`JSON_MAX_BYTES`].
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::client::max_bytes_for;
/// use njt_mcp::limits::{JSON_MAX_BYTES, ZIP_MAX_BYTES};
/// assert_eq!(max_bytes_for("/api/GTFSRT/getGTFS"), ZIP_MAX_BYTES);
/// assert_eq!(max_bytes_for("/api/TrainData/getStationList"), JSON_MAX_BYTES);
/// ```
pub fn max_bytes_for(path: &str) -> usize {
    if path.ends_with("/getGTFS") {
        ZIP_MAX_BYTES
    } else if path.ends_with("/getAlerts")
        || path.ends_with("/getTripUpdates")
        || path.ends_with("/getVehiclePositions")
    {
        PROTO_MAX_BYTES
    } else {
        JSON_MAX_BYTES
    }
}
