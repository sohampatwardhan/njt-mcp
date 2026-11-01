//! stdio MCP session against the synthetic mock.
#![allow(unknown_lints)]
#![deny(clippy::missing_docs_in_private_items)]

#[path = "common/support.rs"]
mod support;

use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde_json::json;
use support::start_mock;

/// The child lists the tool set and reads the synthetic station list.
#[tokio::test]
async fn stdio_lists_tools_and_reads_stations() {
    let server = start_mock().await;
    let dir = std::env::temp_dir().join(format!(
        "njt-mcp-stdio-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_njt-mcp"));
    command
        .arg("stdio")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("NJT_MCP_LOG", "error")
        .env("NJT_ENV", "test")
        .env("NJT_USERNAME", "replace-me")
        .env("NJT_PASSWORD", "replace-me-password")
        .env("NJT_RAIL_ORIGIN", server.uri())
        .env("NJT_BUS_ORIGIN", server.uri())
        .env("NJT_CACHE_DIR", &dir);
    let client =
        ().serve(TokioChildProcess::new(command).expect("spawn"))
            .await
            .expect("initialize");
    let tools = client.peer().list_all_tools().await.expect("tools");
    let mut names: Vec<_> = tools.iter().map(|tool| tool.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "bus_departures",
            "bus_directions",
            "bus_locations",
            "bus_routes",
            "bus_stops",
            "bus_trip_stops",
            "bus_vehicles_near",
            "gtfs_alerts",
            "gtfs_routes",
            "gtfs_stops",
            "gtfs_trip",
            "gtfs_trip_updates",
            "gtfs_vehicle_positions",
            "njt_status",
            "rail_departures",
            "rail_schedule",
            "rail_station_messages",
            "rail_stations",
            "rail_train_stops",
            "rail_vehicles",
        ]
    );
    let result = client
        .call_tool(CallToolRequestParams::new("rail_stations"))
        .await
        .expect("call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .expect("text");
    let value: serde_json::Value = serde_json::from_str(&text.text).expect("json");
    assert_eq!(value["stations"][0]["station"], "EX");
    assert!(!text.text.contains("replace-me-password"));
    let status = client
        .call_tool(CallToolRequestParams::new("njt_status"))
        .await
        .expect("status");
    let status_text = status
        .content
        .first()
        .and_then(|block| block.as_text())
        .expect("status text");
    let status_json: serde_json::Value =
        serde_json::from_str(&status_text.text).expect("status json");
    assert_eq!(status_json["credential_mode"], "password");
    assert!(!status_text.text.contains("minted-token"));
    let _ = json!({});
    let _ = client.cancel().await;
    let _ = std::fs::remove_dir_all(&dir);
}
