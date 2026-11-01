//! Mock NJ Transit origin for integration tests.
//!
//! Responses are synthetic. Nothing here is a captured NJ Transit payload.
//! Not every test binary calls every helper.
#![allow(dead_code)]

use std::path::Path;
use std::time::Duration;

use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use njt_mcp::fixtures::{example_feed_bytes, example_zip};
use njt_mcp::Config;

/// Start a mock that accepts POST multipart calls for every product.
///
/// # Returns
///
/// A server bound to an ephemeral port.
///
/// # Panics
///
/// Panics if the mock cannot bind. Tests cannot continue without it.
///
/// # Examples
///
/// Called from the integration tests in this directory.
pub async fn start_mock() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(respond)
        .mount(&server)
        .await;
    server
}

/// Build a config pointed at the mock.
///
/// # Arguments
///
/// * `origin` - mock base URL.
/// * `dir` - empty cache directory.
/// * `extra` - credential pairs.
///
/// # Returns
///
/// A parsed [`Config`].
///
/// # Panics
///
/// Panics if the pairs are not a valid config. The tests pass valid pairs.
///
/// # Examples
///
/// The service tests pass `NJT_USERNAME` and `NJT_PASSWORD`.
pub fn test_config(origin: &str, dir: &Path, extra: &[(&str, &str)]) -> Config {
    let mut pairs = vec![
        ("NJT_ENV".to_string(), "test".to_string()),
        ("NJT_RAIL_ORIGIN".to_string(), origin.to_string()),
        ("NJT_BUS_ORIGIN".to_string(), origin.to_string()),
        ("NJT_CACHE_DIR".to_string(), dir.display().to_string()),
        ("NJT_RT_TTL".to_string(), "30".to_string()),
    ];
    for (key, value) in extra {
        pairs.push(((*key).to_string(), (*value).to_string()));
    }
    Config::parse(pairs).expect("test config")
}

/// Count requests whose path ends with `suffix`.
///
/// # Arguments
///
/// * `server` - mock server.
/// * `suffix` - path suffix such as `/getToken`.
///
/// # Returns
///
/// The number of matching requests.
///
/// # Panics
///
/// Panics if the mock cannot report requests.
///
/// # Examples
///
/// Used to assert that login is not repeated.
pub async fn hits(server: &MockServer, suffix: &str) -> usize {
    server
        .received_requests()
        .await
        .expect("requests")
        .iter()
        .filter(|request| request.url.path().ends_with(suffix))
        .count()
}

/// Choose a synthetic response from the path and form body.
///
/// # Arguments
///
/// * `request` - recorded mock request.
///
/// # Returns
///
/// A 200 response. Invalid tokens, daily limits, and `Null` are represented as bodies, not HTTP errors.
///
/// # Panics
///
/// This function does not panic. The slow-user login sleeps for 200 milliseconds.
///
/// # Examples
///
/// A body containing `stale-token` returns the invalid-token JSON.
fn respond(request: &Request) -> ResponseTemplate {
    let path = request.url.path().to_string();
    let body = String::from_utf8_lossy(&request.body);
    if path.ends_with("/getToken") || path.ends_with("/authenticateUser") {
        if body.contains("slow-user") {
            std::thread::sleep(Duration::from_millis(200));
        }
        if body.contains("bad-user") {
            return json(r#"{"Authenticated":"False","UserToken":""}"#);
        }
        return json(r#"{"Authenticated":"True","UserToken":"minted-token"}"#);
    }
    if body.contains("stale-token") {
        return json(r#"{"errorMessage":"Invalid token."}"#);
    }
    if path.ends_with("/getStationSchedule") && body.contains("LIMIT") {
        return json(r#"{"errorMessage":"Daily usage limit:5. Your current daily usage: 6"}"#);
    }
    if body.contains("NULLST") {
        return ResponseTemplate::new(200).set_body_string("Null");
    }
    if path.ends_with("/getGTFS") {
        return ResponseTemplate::new(200).set_body_bytes(example_zip());
    }
    if path.ends_with("/getAlerts")
        || path.ends_with("/getTripUpdates")
        || path.ends_with("/getVehiclePositions")
    {
        return ResponseTemplate::new(200).set_body_bytes(example_feed_bytes());
    }
    json(&fixture_json(&path))
}

/// Synthetic JSON for one path.
///
/// # Arguments
///
/// * `path` - request path.
///
/// # Returns
///
/// A JSON string. Unknown paths return an `errorMessage` object.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// `getStationList` returns the example station `EX`.
fn fixture_json(path: &str) -> String {
    let body = if path.ends_with("/getStationList") {
        r#"[{"STATION_2CHAR":"EX","STATIONNAME":"Example Station","STATION_14CHAR":"EXAMPLE","WHEELCHAIR_ACCESSIBLE":"Y"}]"#
    } else if path.ends_with("/getTrainSchedule") || path.ends_with("/getTrainSchedule19Rec") {
        r#"{"STATION_2CHAR":"EX","STATIONNAME":"Example Station","STATIONMSGS":[{"MSG_TEXT":"Example message","MSG_TYPE":"banner","MSG_ID":"1"}],"ITEMS":[{"TRAIN_ID":"9001","DESTINATION":"Example Terminal","TRACK":"1","LINE":"Example Line","STATUS":"ON TIME","SEC_LATE":"0","SCHED_DEP_DATE":"10:00","STOPS":[{"STATION_2CHAR":"EX","STATIONNAME":"Example Station","TIME":"10:00"}]}]}"#
    } else if path.ends_with("/getStationMSG") {
        r#"[{"MSG_TYPE":"banner","MSG_TEXT":"Example message","MSG_ID":"1"}]"#
    } else if path.ends_with("/getTrainStopList") {
        r#"{"TRAIN_ID":"9001","DESTINATION":"Example Terminal","STOPS":[{"STATION_2CHAR":"EX","STATIONNAME":"Example Station","TIME":"10:00"}]}"#
    } else if path.ends_with("/getVehicleData") {
        r#"[{"ID":"9001","TRAIN_LINE":"Example Line","DIRECTION":"E","SEC_LATE":"30","NEXT_STOP":"Example Terminal","LATITUDE":"40.0","LONGITUDE":"-74.0"}]"#
    } else if path.ends_with("/getStationSchedule") {
        r#"[{"STATION_2CHAR":"EX","STATIONNAME":"Example Station","ITEMS":[{"TRAIN_ID":"9001","DESTINATION":"Example Terminal","LINE":"Example Line","SCHED_DEP_DATE":"10:00"}]}]"#
    } else if path.ends_with("/getLocations") {
        r#"[{"bus_terminal_code":"EX","bus_terminal_name":"Example Terminal"}]"#
    } else if path.ends_with("/getBusRoutes") {
        r#"[{"BusRouteID":"99","BusRouteDescription":"Example Route"}]"#
    } else if path.ends_with("/getBusDirectionsData") {
        r#"[{"Direction_1":"To Example","Direction_2":"To Other"}]"#
    } else if path.ends_with("/getStops") {
        r#"[{"busstopnumber":"10001","busstopdescription":"Example Stop"}]"#
    } else if path.ends_with("/getStopName") {
        r#"{"stopName":"Example Stop"}"#
    } else if path.ends_with("/getBusLocationsData") {
        r#"[{"busstopnumber":"10001","busstopdescription":"Example Stop","latitude":"40.0","longitude":"-74.0","distance":"12","modetype":"BUS"}]"#
    } else if path.ends_with("/getBusDV") || path.ends_with("/getRouteTrips") {
        r#"{"DVTrip":[{"public_route":"99","header":"Example","departuretime":"10:00","internal_trip_number":"INT1","sched_dep_time":"6/22/2023 12:50:00 AM","timing_point_id":"TP1"}]}"#
    } else if path.ends_with("/getTripStops") {
        r#"[{"TripNumber":"INT1","Description":"Example Stop","StopID":"10001","SchedDepTime":"10:00"}]"#
    } else if path.ends_with("/getVehicleLocations") {
        r#"[{"VehicleID":"V1","VehicleRoute":"99","VehicleLat":"40.1","VehicleLong":"-74.1","VehicleSecondsLate":"15"}]"#
    } else {
        r#"{"errorMessage":"unknown test path"}"#
    };
    body.to_string()
}

/// JSON response template.
///
/// # Arguments
///
/// * `body` - response text.
///
/// # Returns
///
/// A 200 template with a JSON content type.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Login success and the station list both use this helper.
fn json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_string(body)
}
