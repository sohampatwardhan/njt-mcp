//! `njt-mcp` binary.
//!
//! `stdio` speaks MCP on standard input and output. `http` serves streamable
//! HTTP at `/mcp` and liveness at `/healthz`. Credentials are read from the
//! environment by [`njt_mcp::Config::from_env`]. A missing credential set exits
//! with status 1 before the server listens.
//!
//! # Examples
//!
//! ```text
//! NJT_TOKEN=replace-me njt-mcp stdio
//! NJT_USERNAME=replace-me NJT_PASSWORD=replace-me njt-mcp http
//! ```

#![allow(unknown_lints)]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(rustdoc::missing_errors_doc)]
#![deny(rustdoc::missing_panics_doc)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

use std::net::SocketAddr;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use njt_mcp::{serve_http, serve_stdio, Config, NjtServer, Service};

/// Command line.
#[derive(Parser)]
#[command(
    name = "njt-mcp",
    version,
    about = "MCP server for NJ Transit rail, bus, and light rail"
)]
/// `Cli`.
///
/// # Examples
///
/// Constructed inside this module.
struct Cli {
    /// stdio or HTTP.
    #[command(subcommand)]
    command: Command,
}

/// Transports.
#[derive(Subcommand)]
enum Command {
    /// Speak MCP on stdin and stdout. Logs go to stderr.
    Stdio,
    /// Serve streamable HTTP MCP.
    Http {
        /// Listen address. Defaults to 127.0.0.1:8092. `NJT_MCP_BIND` overrides it.
        #[arg(long, env = "NJT_MCP_BIND", default_value = njt_mcp::limits::DEFAULT_BIND)]
        bind: SocketAddr,
        /// Extra Host header to allow, comma-separated in the environment.
        #[arg(
            long = "allowed-host",
            env = "NJT_MCP_ALLOWED_HOST",
            value_delimiter = ','
        )]
        allowed_hosts: Vec<String>,
    },
}

/// Load configuration, then serve until the client disconnects or Ctrl-C.
///
/// # Panics
///
/// This function does not panic. Configuration and runtime failures exit the process.
///
/// # Examples
///
/// The process is started by an MCP client. See the crate README.
fn main() {
    let cli = Cli::parse();
    init_tracing(matches!(cli.command, Command::Http { .. }));
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("njt-mcp: {error}");
            std::process::exit(1);
        }
    };
    let service = match Service::new(config) {
        Ok(service) => Arc::new(service),
        Err(error) => {
            eprintln!("njt-mcp: {error}");
            std::process::exit(1);
        }
    };
    let result = match cli.command {
        Command::Stdio => {
            let runtime = tokio::runtime::Runtime::new();
            match runtime {
                Ok(runtime) => runtime.block_on(serve_stdio(NjtServer::new(service))),
                Err(error) => Err(njt_mcp::AppError::new(format!("runtime: {error}"))),
            }
        }
        Command::Http {
            bind,
            allowed_hosts,
        } => {
            let runtime = tokio::runtime::Runtime::new();
            match runtime {
                Ok(runtime) => runtime.block_on(serve_http(service, bind, allowed_hosts)),
                Err(error) => Err(njt_mcp::AppError::new(format!("runtime: {error}"))),
            }
        }
    };
    if let Err(error) = result {
        eprintln!("njt-mcp: {error}");
        std::process::exit(1);
    }
}

/// Start stderr tracing. stdio defaults to `warn` so stdout stays clean.
///
/// # Arguments
///
/// * `http` - whether the HTTP command is selected.
///
/// # Panics
///
/// This function does not panic. A second init is ignored.
///
/// # Examples
///
/// Called once from [`main`].
fn init_tracing(http: bool) {
    let fallback = if http { "info" } else { "warn" };
    let filter = std::env::var("NJT_MCP_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| fallback.to_string());
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}
