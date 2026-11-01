//! Streamable HTTP MCP against the synthetic mock.
#![allow(unknown_lints)]
#![deny(clippy::missing_docs_in_private_items)]

#[path = "common/support.rs"]
mod support;

use std::sync::Arc;

use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::json;

use njt_mcp::{spawn_http, Service};
use support::{start_mock, test_config};

/// `/healthz` answers and `bus_routes` returns the synthetic route.
#[tokio::test]
async fn http_health_and_bus_routes() {
    let upstream = start_mock().await;
    let dir = std::env::temp_dir().join(format!(
        "njt-mcp-http-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let config = test_config(
        &upstream.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "replace-me"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Arc::new(Service::new(config).expect("service"));
    let (addr, handle) = spawn_http(service, "127.0.0.1:0".parse().expect("addr"), Vec::new())
        .await
        .expect("bind");
    let health = reqwest::Client::new()
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .expect("health");
    assert!(health.status().is_success());
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://{addr}/mcp"
        )))
        .await
        .expect("initialize");
    let tools = client.peer().list_all_tools().await.expect("tools");
    assert_eq!(tools.len(), 20);
    let result = client
        .call_tool(
            CallToolRequestParams::new("bus_routes")
                .with_arguments(json!({"mode": "BUS"}).as_object().expect("object").clone()),
        )
        .await
        .expect("routes");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .expect("text");
    let value: serde_json::Value = serde_json::from_str(&text.text).expect("json");
    assert_eq!(value["routes"][0]["description"], "Example Route");
    assert!(!text.text.contains("replace-me-password"));
    let _ = client.cancel().await;
    handle.abort();
    let _ = std::fs::remove_dir_all(&dir);
}
