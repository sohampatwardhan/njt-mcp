//! Service behavior against the synthetic mock.
#![allow(unknown_lints)]
#![deny(clippy::missing_docs_in_private_items)]

#[path = "common/support.rs"]
mod support;

use std::sync::Arc;

use njt_mcp::service::{BusDepartureQuery, BusStopQuery};
use njt_mcp::store::Store;
use njt_mcp::{Api, Service, Transit};
use support::{hits, start_mock, test_config};

/// Unique cache directory for one test.
///
/// # Arguments
///
/// * `label` - short name mixed into the path.
///
/// # Returns
///
/// A path that did not exist.
///
/// # Panics
///
/// Panics if the clock is before the Unix epoch.
///
/// # Examples
///
/// Each test in this file calls it once.
fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "njt-mcp-it-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Password mode mints once, serves the station list, and writes a mode 0600 token file.
#[tokio::test]
async fn password_mode_mints_once_and_locks_down_the_token_file() {
    let server = start_mock().await;
    let dir = temp_dir("mint");
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "replace-me"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Service::new(config).expect("service");
    let stations = service.rail_stations().await.expect("stations");
    assert_eq!(stations["stations"][0]["station"], "EX");
    assert_eq!(stations["stations"][0]["wheelchair"], true);
    let again = service.rail_stations().await.expect("cached");
    assert_eq!(again["count"], 1);
    assert_eq!(hits(&server, "/getToken").await, 1);
    assert_eq!(hits(&server, "/getStationList").await, 1);
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(dir.join("state.json"))
            .expect("state")
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode, 0o600);
    drop(service);
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "replace-me"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let restarted = Service::new(config).expect("restart");
    restarted.rail_stations().await.expect("restart stations");
    assert_eq!(hits(&server, "/getToken").await, 1);
    let rendered = restarted.status().to_string();
    assert!(!rendered.contains("replace-me-password"));
    assert!(!rendered.contains("minted-token"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Concurrent first calls share a single login.
#[tokio::test]
async fn concurrent_calls_mint_once() {
    let server = start_mock().await;
    let dir = temp_dir("flight");
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "slow-user"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Arc::new(Service::new(config).expect("service"));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let service = Arc::clone(&service);
        tasks.push(tokio::spawn(async move {
            service.rail_stations().await.expect("stations")
        }));
    }
    for task in tasks {
        task.await.expect("join");
    }
    assert_eq!(hits(&server, "/getToken").await, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stale `NJT_TOKEN` is replaced when a password is also configured.
#[tokio::test]
async fn token_falls_back_to_password_after_invalid_token() {
    let server = start_mock().await;
    let dir = temp_dir("fallback");
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_TOKEN", "stale-token"),
            ("NJT_USERNAME", "replace-me"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Service::new(config).expect("service");
    let stations = service.rail_stations().await.expect("stations");
    assert_eq!(stations["stations"][0]["name"], "Example Station");
    assert_eq!(hits(&server, "/getToken").await, 1);
    assert_eq!(hits(&server, "/getStationList").await, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Token-only mode reports expiry and does not call login.
#[tokio::test]
async fn token_only_mode_reports_expiry() {
    let server = start_mock().await;
    let dir = temp_dir("expired");
    let config = test_config(&server.uri(), &dir, &[("NJT_TOKEN", "stale-token")]);
    let service = Service::new(config).expect("service");
    let error = service.rail_stations().await.expect_err("expired");
    let text = error.to_string();
    assert!(text.contains("expired"), "{text}");
    assert!(text.contains("NJT_TOKEN"), "{text}");
    assert!(!text.contains("stale-token"), "{text}");
    assert_eq!(hits(&server, "/getToken").await, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rejected password is not retried and is not copied into the error.
#[tokio::test]
async fn rejected_password_is_sticky_and_redacted() {
    let server = start_mock().await;
    let dir = temp_dir("login");
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "bad-user"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Service::new(config).expect("service");
    let error = service.rail_stations().await.expect_err("login");
    assert!(error.to_string().contains("Authenticated"));
    assert!(!error.to_string().contains("replace-me-password"));
    let _ = service.rail_stations().await.expect_err("sticky");
    assert_eq!(hits(&server, "/getToken").await, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The daily-limit body is returned and the path is not called again.
#[tokio::test]
async fn daily_limit_is_reported_and_not_retried() {
    let server = start_mock().await;
    let dir = temp_dir("limit");
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "replace-me"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Service::new(config).expect("service");
    let error = service
        .rail_schedule("LIMIT", false, 0, 20)
        .await
        .expect_err("limit");
    assert!(error.to_string().contains("usage: 6"), "{}", error);
    let _ = service
        .rail_schedule("LIMIT", false, 0, 20)
        .await
        .expect_err("blocked");
    assert_eq!(hits(&server, "/getStationSchedule").await, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A literal Null body becomes a parameter error.
#[tokio::test]
async fn null_body_is_a_parameter_error() {
    let server = start_mock().await;
    let dir = temp_dir("null");
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "replace-me"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Service::new(config).expect("service");
    let error = service
        .rail_departures("NULLST", None, 4)
        .await
        .expect_err("null");
    assert!(error.to_string().contains("Null"), "{}", error);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same station schedule is fetched once and then served from disk.
#[tokio::test]
async fn station_schedule_is_cached_for_the_day() {
    let server = start_mock().await;
    let dir = temp_dir("sched");
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "replace-me"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Service::new(config).expect("service");
    let first = service
        .rail_schedule("EX", false, 0, 20)
        .await
        .expect("first");
    assert_eq!(first["cached"], false);
    assert_eq!(first["items"][0]["train"], "9001");
    let second = service
        .rail_schedule("EX", false, 0, 20)
        .await
        .expect("second");
    assert_eq!(second["cached"], true);
    assert_eq!(hits(&server, "/getStationSchedule").await, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Four upstream schedule calls are the local cap.
#[tokio::test]
async fn schedule_cap_stops_before_the_fifth_upstream_call() {
    let server = start_mock().await;
    let dir = temp_dir("cap");
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "replace-me"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Service::new(config).expect("service");
    for code in ["AA", "BB", "CC", "DD"] {
        service
            .rail_schedule(code, false, 0, 5)
            .await
            .unwrap_or_else(|error| panic!("{code}: {error}"));
    }
    let error = service
        .rail_schedule("EE", false, 0, 5)
        .await
        .expect_err("cap");
    assert!(error.to_string().contains("getStationSchedule"), "{error}");
    assert_eq!(hits(&server, "/getStationSchedule").await, 4);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bus, GTFS, and GTFS-Realtime tools decode the synthetic payloads.
#[tokio::test]
async fn bus_gtfs_and_realtime_tools_decode_fixtures() {
    let server = start_mock().await;
    let dir = temp_dir("bus");
    let config = test_config(
        &server.uri(),
        &dir,
        &[
            ("NJT_USERNAME", "replace-me"),
            ("NJT_PASSWORD", "replace-me-password"),
        ],
    );
    let service = Service::new(config).expect("service");
    let routes = service.bus_routes(Some("BUS")).await.expect("routes");
    assert_eq!(routes["routes"][0]["route"], "99");
    let directions = service.bus_directions("99").await.expect("directions");
    assert_eq!(directions["directions"][0], "To Example");
    let stops = service
        .bus_stops(&BusStopQuery {
            route: Some("99".into()),
            direction: Some("To Example".into()),
            name_contains: None,
            stop: None,
            latitude: None,
            longitude: None,
            radius: None,
            mode: None,
            limit: 20,
            offset: 0,
        })
        .await
        .expect("stops");
    assert_eq!(stops["stops"][0]["stop"], "10001");
    let board = BusDepartureQuery {
        stop: Some("10001".into()),
        ..BusDepartureQuery::default()
    };
    let departures = service.bus_departures(&board).await.expect("board");
    assert_eq!(departures["departures"][0]["internal_trip_number"], "INT1");
    let trip_stops = service
        .bus_trip_stops("TP1", "6/22/2023 12:50:00 AM", "INT1")
        .await
        .expect("trip stops");
    assert_eq!(trip_stops["stops"][0]["stop"], "10001");
    let near = service
        .bus_vehicles_near("40.0", "-74.0", 1000, Some("BUS"))
        .await
        .expect("near");
    assert_eq!(near["vehicles"][0]["sec_late"], 15);
    let gtfs_stops = service
        .gtfs_stops(Transit::Rail, "Example", 10, 0)
        .await
        .expect("gtfs stops");
    assert_eq!(gtfs_stops["stops"][0]["stop_id"], "EX1");
    let trip = service
        .gtfs_trip(Transit::Rail, "9001", 10, 0)
        .await
        .expect("gtfs trip");
    assert_eq!(trip["internal_trip_number"], "INT1");
    let updates = service
        .gtfs_trip_updates(&njt_mcp::realtime::RtQuery {
            transit: Transit::Rail,
            route: Some("R1".into()),
            trip: None,
            stop: None,
            text: None,
            limit: 10,
            offset: 0,
            stop_limit: 4,
            max_chars: 200,
        })
        .await
        .expect("updates");
    assert_eq!(updates["trip_updates"][0]["trip_id"], "T1");
    assert_eq!(hits(&server, "/getGTFS").await, 1);
    service
        .gtfs_stops(Transit::Rail, "EX2", 10, 0)
        .await
        .expect("cached zip");
    assert_eq!(hits(&server, "/getGTFS").await, 1);
    let _ = Store::open(&dir).expect("store still opens");
    assert_eq!(service.status()["tokens_cached"]["bus_dv2"], true);
    assert_eq!(
        service.status()["tokens_cached"][Api::GtfsRt.as_str()],
        true
    );
    let _ = std::fs::remove_dir_all(&dir);
}
