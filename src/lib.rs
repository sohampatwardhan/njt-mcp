//! MCP server for NJ Transit rail, bus, and light rail.
//!
//! The binary speaks MCP on stdio and on streamable HTTP. Tools call NJ
//! Transit's form APIs and decode GTFS-Realtime protobufs. Static GTFS zips
//! are cached on disk and searched locally.
//!
//! ## Modules
//!
//! - [`api`] — products (`TrainData`, `GTFSRT`, `GTFSG2`, `BUSDV2`) and rail/bus GTFS choice
//! - [`auth`] — per-API tokens, single-flight login, token-only expiry
//! - [`client`] — multipart POST
//! - [`config`] — environment variables
//! - [`error`] — agent-facing errors that do not contain secrets
//! - [`fixtures`] — synthetic zip and protobuf used by tests
//! - [`gtfs`] — minimal static GTFS parser
//! - [`gtfs_rt`] — vendored GTFS-Realtime protobuf types
//! - [`limits`] — local quotas and the New York service day
//! - [`mcp`] — tool list and transports
//! - [`realtime`] — protobuf decoding and filters
//! - [`service`] — tool implementations
//! - [`shape`] — compact JSON
//! - [`store`] — mode `0600` token, schedule, and zip cache
//! - [`upstream`] — `errorMessage`, login, and `Null` classification
//!
//! ## Credentials
//!
//! Set `NJT_TOKEN`, or both `NJT_USERNAME` and `NJT_PASSWORD`. When both forms
//! are set, `NJT_TOKEN` is tried first and a rejection mints a token for that
//! API. Token-only mode cannot refresh.
//!
//! # Examples
//!
//! ```
//! use njt_mcp::{Config, CredentialMode};
//! let config = Config::parse([
//!     ("NJT_TOKEN", "replace-me"),
//!     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
//! ])
//! .unwrap();
//! assert_eq!(config.credential_mode(), CredentialMode::TokenOnly);
//! ```
#![allow(unknown_lints)]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(rustdoc::missing_errors_doc)]
#![deny(rustdoc::missing_panics_doc)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

pub mod api;
pub mod auth;
pub mod client;
pub mod config;
pub mod error;
pub mod fixtures;
pub mod gtfs;
pub mod limits;
pub mod mcp;
pub mod realtime;
pub mod service;
pub mod shape;
pub mod store;
pub mod upstream;

pub mod gtfs_rt {
    //! Vendored GTFS-Realtime schema.
    //!
    //! The messages are generated from `proto/gtfs-realtime.proto` (Apache-2.0,
    //! MobilityData / Google). Prost copies the protobuf comments onto the
    //! generated items. A few generated helpers do not carry rustdoc, so this
    //! module allows the documentation lints that the rest of the crate denies.
    #![allow(missing_docs)]
    #![allow(clippy::missing_docs_in_private_items)]
    #![allow(clippy::doc_lazy_continuation)]
    #![allow(clippy::doc_overindented_list_items)]
    #![allow(rustdoc::missing_errors_doc)]
    #![allow(rustdoc::missing_panics_doc)]
    #![allow(rustdoc::broken_intra_doc_links)]
    #![allow(rustdoc::private_intra_doc_links)]
    include!(concat!(env!("OUT_DIR"), "/transit_realtime.rs"));
}

pub use api::{Api, OriginHost, Transit};
pub use config::{Config, ConfigError, CredentialMode, Credentials, Environment};
pub use error::{redact, redact_error, AppError, ErrorKind};
pub use limits::{now_unix, service_day};
pub use mcp::{serve_http, serve_stdio, spawn_http, NjtServer};
pub use service::Service;
