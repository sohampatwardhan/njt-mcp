//! Compact JSON for tool results.
//!
//! NJ Transit field names stay recognizable (`station`, `train`, `sec_late`)
//! but UI colors, rich text, and empty strings are dropped. Numbers that arrive
//! as strings are parsed when they are plain integers or decimals.

use serde_json::{json, Value};

/// Page of JSON rows.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::page;
/// use serde_json::json;
/// let paged = page(vec![json!(1), json!(2), json!(3)], 1, 1);
/// assert_eq!(paged.total, 3);
/// assert_eq!(paged.next_offset, Some(2));
/// assert_eq!(paged.rows, vec![json!(2)]);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// Rows after `offset` and `limit`.
    pub rows: Vec<Value>,
    /// Count before paging.
    pub total: usize,
    /// Next `offset`, or `None` on the last page.
    pub next_offset: Option<usize>,
}

/// Slice `rows` without panicking on a large offset.
///
/// # Arguments
///
/// * `rows` - full list.
/// * `offset` - rows to skip.
/// * `limit` - maximum rows to keep. Zero keeps nothing.
///
/// # Returns
///
/// A [`Page`].
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::page;
/// let paged = page(Vec::<serde_json::Value>::new(), 5, 10);
/// assert_eq!(paged.total, 0);
/// assert!(paged.next_offset.is_none());
/// ```
pub fn page(rows: Vec<Value>, offset: usize, limit: usize) -> Page {
    let total = rows.len();
    let rows = rows.into_iter().skip(offset).take(limit).collect();
    let next_offset = offset.checked_add(limit).filter(|end| *end < total);
    Page {
        rows,
        total,
        next_offset,
    }
}

/// Clamp a tool limit into `1..=max`.
///
/// # Arguments
///
/// * `value` - requested limit. `None` uses `default`.
/// * `default` - used when `value` is missing. Values outside the range are clamped.
/// * `max` - inclusive maximum. Zero is treated as one.
///
/// # Returns
///
/// The clamped limit as `usize`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::clamp_limit;
/// assert_eq!(clamp_limit(None, 20, 100), 20);
/// assert_eq!(clamp_limit(Some(0), 20, 100), 1);
/// assert_eq!(clamp_limit(Some(500), 20, 100), 100);
/// ```
pub fn clamp_limit(value: Option<u32>, default: u32, max: u32) -> usize {
    let max = max.max(1);
    let chosen = value.unwrap_or(default);
    let chosen = chosen.clamp(1, max);
    usize::try_from(chosen).unwrap_or(max as usize)
}

/// Require a short station code and normalize it to uppercase.
///
/// # Arguments
///
/// * `station` - value from `rail_stations`, usually `STATION_2CHAR`.
///
/// # Returns
///
/// The uppercase code.
///
/// # Errors
///
/// Returns [`crate::AppError`] when the value is empty, longer than 32 characters,
/// or contains characters other than ASCII letters, digits, `_`, and `-`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::station_key;
/// assert_eq!(station_key(" ex ").unwrap(), "EX");
/// assert!(station_key("../etc").is_err());
/// ```
pub fn station_key(station: &str) -> Result<String, crate::AppError> {
    let trimmed = station.trim();
    let ok = !trimmed.is_empty()
        && trimmed.len() <= 32
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if ok {
        Ok(trimmed.to_ascii_uppercase())
    } else {
        Err(crate::AppError::new(
            "station must be a short code from rail_stations (letters, digits, _ or -)",
        ))
    }
}

/// Shape `getStationList`.
///
/// # Arguments
///
/// * `value` - JSON array of station objects, or one object.
///
/// # Returns
///
/// `{stations, count}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::stations;
/// let value = stations(&serde_json::json!([{
///     "STATION_2CHAR": "EX",
///     "STATIONNAME": "Example Station",
///     "STATION_14CHAR": "EXAMPLE",
///     "WHEELCHAIR_ACCESSIBLE": "Y"
/// }]));
/// assert_eq!(value["stations"][0]["station"], "EX");
/// assert_eq!(value["stations"][0]["wheelchair"], true);
/// assert_eq!(value["count"], 1);
/// ```
pub fn stations(value: &Value) -> Value {
    let rows = as_list(value)
        .into_iter()
        .map(|row| {
            json!({
                "station": text(row, "STATION_2CHAR"),
                "name": text(row, "STATIONNAME"),
                "station_14": text(row, "STATION_14CHAR"),
                "wheelchair": wheelchair(text(row, "WHEELCHAIR_ACCESSIBLE")),
            })
        })
        .collect::<Vec<_>>();
    json!({"stations": rows, "count": rows.len()})
}

/// Shape `getStationMSG`.
///
/// # Arguments
///
/// * `value` - JSON array of messages.
/// * `station` - station code that was requested, or empty.
/// * `line` - line that was requested, or empty.
///
/// # Returns
///
/// `{station, line, messages}`. Message text is clipped to 400 characters.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::station_messages;
/// let value = station_messages(&serde_json::json!([{
///     "MSG_TYPE": "banner",
///     "MSG_TEXT": "Example message",
///     "MSG_ID": "1"
/// }]), "EX", "");
/// assert_eq!(value["messages"][0]["text"], "Example message");
/// ```
pub fn station_messages(value: &Value, station: &str, line: &str) -> Value {
    let rows = as_list(value)
        .into_iter()
        .map(|row| {
            json!({
                "type": empty_null(text(row, "MSG_TYPE")),
                "text": clip(&text(row, "MSG_TEXT"), 400),
                "id": empty_null(text(row, "MSG_ID")),
                "agency": empty_null(text(row, "MSG_AGENCY")),
                "published": empty_null(first_nonempty(&[text(row, "MSG_PUBDATE_UTC"), text(row, "MSG_PUBDATE")])),
                "url": empty_null(text(row, "MSG_URL")),
            })
        })
        .collect::<Vec<_>>();
    json!({"station": station, "line": line, "messages": rows})
}

/// Shape `getTrainSchedule` or `getTrainSchedule19Rec`.
///
/// # Arguments
///
/// * `value` - `stationinfo` object.
/// * `line_filter` - line passed to the 19-record call, if any.
/// * `stop_limit` - stops kept on each departure.
///
/// # Returns
///
/// Station identity, clipped messages, and departures.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::departures;
/// let value = departures(&serde_json::json!({
///     "STATION_2CHAR": "EX",
///     "STATIONNAME": "Example Station",
///     "ITEMS": [{
///         "TRAIN_ID": "9001",
///         "DESTINATION": "Example Terminal",
///         "TRACK": "1",
///         "LINE": "Example Line",
///         "STATUS": "ON TIME",
///         "SEC_LATE": "0",
///         "STOPS": [{"STATION_2CHAR": "EX", "STATIONNAME": "Example Station", "TIME": "10:00"}]
///     }]
/// }), None, 4);
/// assert_eq!(value["departures"][0]["train"], "9001");
/// assert_eq!(value["departures"][0]["sec_late"], 0);
/// ```
pub fn departures(value: &Value, line_filter: Option<&str>, stop_limit: usize) -> Value {
    let messages = station_messages(
        value.get("STATIONMSGS").unwrap_or(&Value::Null),
        &text(value, "STATION_2CHAR"),
        line_filter.unwrap_or(""),
    );
    let rows = as_list(value.get("ITEMS").unwrap_or(value))
        .into_iter()
        .map(|row| departure_row(row, stop_limit))
        .collect::<Vec<_>>();
    json!({
        "station": text(value, "STATION_2CHAR"),
        "name": text(value, "STATIONNAME"),
        "line": line_filter,
        "messages": messages.get("messages").cloned().unwrap_or(Value::Array(Vec::new())),
        "departures": rows,
    })
}

/// Shape `getTrainStopList`.
///
/// # Arguments
///
/// * `value` - `stops` object from TrainData.
///
/// # Returns
///
/// Train identity and the stop list. Colors are omitted.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::train_stops;
/// let value = train_stops(&serde_json::json!({
///     "TRAIN_ID": "9001",
///     "DESTINATION": "Example Terminal",
///     "STOPS": [{"STATION_2CHAR": "EX", "STATIONNAME": "Example Station", "TIME": "10:00"}]
/// }));
/// assert_eq!(value["train"], "9001");
/// assert_eq!(value["stops"][0]["station"], "EX");
/// ```
pub fn train_stops(value: &Value) -> Value {
    let stops = as_list(value.get("STOPS").unwrap_or(&Value::Null))
        .into_iter()
        .map(stop_row)
        .collect::<Vec<_>>();
    json!({
        "train": text(value, "TRAIN_ID"),
        "line_code": empty_null(text(value, "LINECODE")),
        "destination": empty_null(text(value, "DESTINATION")),
        "transfer_at": empty_null(text(value, "TRANSFERAT")),
        "stops": stops,
    })
}

/// Shape `getVehicleData`, optionally filtered.
///
/// # Arguments
///
/// * `value` - JSON array of vehicle rows.
/// * `train` - optional train id filter.
/// * `line` - optional line filter, case-insensitive substring.
/// * `offset` - rows to skip after filtering.
/// * `limit` - page size.
///
/// # Returns
///
/// `{vehicles, total, next_offset}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::rail_vehicles;
/// let value = rail_vehicles(&serde_json::json!([{
///     "ID": "9001",
///     "TRAIN_LINE": "Example Line",
///     "DIRECTION": "E",
///     "SEC_LATE": "30",
///     "NEXT_STOP": "Example Terminal",
///     "LATITUDE": "40.0",
///     "LONGITUDE": "-74.0"
/// }]), None, None, 0, 20);
/// assert_eq!(value["vehicles"][0]["sec_late"], 30);
/// assert_eq!(value["vehicles"][0]["latitude"], 40.0);
/// ```
pub fn rail_vehicles(
    value: &Value,
    train: Option<&str>,
    line: Option<&str>,
    offset: usize,
    limit: usize,
) -> Value {
    let rows = as_list(value)
        .into_iter()
        .filter(|row| {
            let id = text(row, "ID");
            let row_line = text(row, "TRAIN_LINE");
            let train_ok = train
                .map(|wanted| id.eq_ignore_ascii_case(wanted.trim()))
                .unwrap_or(true);
            let line_ok = line
                .map(|wanted| {
                    row_line
                        .to_ascii_lowercase()
                        .contains(&wanted.trim().to_ascii_lowercase())
                })
                .unwrap_or(true);
            train_ok && line_ok
        })
        .map(|row| {
            json!({
                "train": text(row, "ID"),
                "line": empty_null(text(row, "TRAIN_LINE")),
                "direction": empty_null(text(row, "DIRECTION")),
                "sec_late": number_or_text(&text(row, "SEC_LATE")),
                "next_stop": empty_null(text(row, "NEXT_STOP")),
                "latitude": coord(&text(row, "LATITUDE")),
                "longitude": coord(&text(row, "LONGITUDE")),
                "scheduled": empty_null(text(row, "SCHED_DEP_TIME")),
                "updated": empty_null(text(row, "LAST_MODIFIED")),
            })
        })
        .collect::<Vec<_>>();
    let paged = page(rows, offset, limit);
    json!({
        "vehicles": paged.rows,
        "total": paged.total,
        "next_offset": paged.next_offset,
    })
}

/// Shape `getStationSchedule`.
///
/// # Arguments
///
/// * `value` - array of `dailystationinfo`, or one object.
/// * `station` - requested code.
/// * `njt_only` - flag sent as `NJTOnly`.
/// * `cached` - whether the body came from today's disk cache.
/// * `day` - New York service day of the cache.
/// * `offset` - departure rows to skip.
/// * `limit` - page size.
///
/// # Returns
///
/// Flattened schedule rows plus paging fields.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::station_schedule;
/// let value = station_schedule(&serde_json::json!([{
///     "STATION_2CHAR": "EX",
///     "STATIONNAME": "Example Station",
///     "ITEMS": [{"TRAIN_ID": "9001", "DESTINATION": "Example Terminal", "LINE": "Example Line", "SCHED_DEP_DATE": "10:00"}]
/// }]), "EX", false, true, "2026-01-15", 0, 20);
/// assert_eq!(value["cached"], true);
/// assert_eq!(value["items"][0]["train"], "9001");
/// ```
pub fn station_schedule(
    value: &Value,
    station: &str,
    njt_only: bool,
    cached: bool,
    day: &str,
    offset: usize,
    limit: usize,
) -> Value {
    let mut name = String::new();
    let mut items = Vec::new();
    for station_row in as_list(value) {
        if name.is_empty() {
            name = text(station_row, "STATIONNAME");
        }
        for item in as_list(station_row.get("ITEMS").unwrap_or(&Value::Null)) {
            items.push(json!({
                "train": text(item, "TRAIN_ID"),
                "destination": empty_null(text(item, "DESTINATION")),
                "track": empty_null(text(item, "TRACK")),
                "line": empty_null(text(item, "LINE")),
                "scheduled": empty_null(text(item, "SCHED_DEP_DATE")),
                "direction": empty_null(text(item, "DIRECTION")),
                "connecting_train": empty_null(text(item, "CONNECTING_TRAIN_ID")),
                "position": empty_null(text(item, "STATION_POSITION")),
                "pickup": empty_null(text(item, "PERM_PICKUP")),
                "dropoff": empty_null(text(item, "PERM_DROPOFF")),
                "stop_code": empty_null(text(item, "STOP_CODE")),
            }));
        }
    }
    let paged = page(items, offset, limit);
    json!({
        "station": station,
        "name": name,
        "njt_only": njt_only,
        "cached": cached,
        "day": day,
        "items": paged.rows,
        "total": paged.total,
        "next_offset": paged.next_offset,
    })
}

/// Shape `getLocations`.
///
/// # Arguments
///
/// * `value` - JSON array.
/// * `mode` - mode that was requested.
///
/// # Returns
///
/// `{mode, locations}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::bus_locations;
/// let value = bus_locations(&serde_json::json!([{
///     "bus_terminal_code": "EX",
///     "bus_terminal_name": "Example Terminal"
/// }]), "ALL");
/// assert_eq!(value["locations"][0]["code"], "EX");
/// ```
pub fn bus_locations(value: &Value, mode: &str) -> Value {
    let rows = as_list(value)
        .into_iter()
        .map(|row| {
            json!({
                "code": text(row, "bus_terminal_code"),
                "name": text(row, "bus_terminal_name"),
            })
        })
        .collect::<Vec<_>>();
    json!({"mode": mode, "locations": rows})
}

/// Shape `getBusRoutes`.
///
/// # Arguments
///
/// * `value` - JSON array.
/// * `mode` - mode that was requested.
///
/// # Returns
///
/// `{mode, routes}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::bus_routes;
/// let value = bus_routes(&serde_json::json!([{
///     "BusRouteID": "99",
///     "BusRouteDescription": "Example Route"
/// }]), "BUS");
/// assert_eq!(value["routes"][0]["route"], "99");
/// ```
pub fn bus_routes(value: &Value, mode: &str) -> Value {
    let rows = as_list(value)
        .into_iter()
        .map(|row| {
            json!({
                "route": text(row, "BusRouteID"),
                "description": text(row, "BusRouteDescription"),
            })
        })
        .collect::<Vec<_>>();
    json!({"mode": mode, "routes": rows})
}

/// Shape `getBusDirectionsData`.
///
/// # Arguments
///
/// * `value` - JSON array of direction objects.
/// * `route` - route that was requested.
///
/// # Returns
///
/// `{route, directions}` with empty direction strings removed.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::bus_directions;
/// let value = bus_directions(&serde_json::json!([{
///     "Direction_1": "To Example",
///     "Direction_2": "To Other"
/// }]), "99");
/// assert_eq!(value["directions"], serde_json::json!(["To Example", "To Other"]));
/// ```
pub fn bus_directions(value: &Value, route: &str) -> Value {
    let mut directions = Vec::new();
    for row in as_list(value) {
        for key in ["Direction_1", "Direction_2"] {
            let text = text(row, key);
            if !text.is_empty() {
                directions.push(Value::String(text));
            }
        }
    }
    json!({"route": route, "directions": directions})
}

/// Shape `getStops`.
///
/// # Arguments
///
/// * `value` - JSON array.
/// * `route` - route filter.
/// * `direction` - direction filter.
///
/// # Returns
///
/// `{source, route, direction, stops}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::bus_stops;
/// let value = bus_stops(&serde_json::json!([{
///     "busstopnumber": "10001",
///     "busstopdescription": "Example Stop"
/// }]), "99", "To Example");
/// assert_eq!(value["stops"][0]["stop"], "10001");
/// ```
pub fn bus_stops(value: &Value, route: &str, direction: &str) -> Value {
    let rows = as_list(value)
        .into_iter()
        .map(|row| {
            json!({
                "stop": text(row, "busstopnumber"),
                "name": text(row, "busstopdescription"),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "source": "stops",
        "route": route,
        "direction": direction,
        "stops": rows,
    })
}

/// Shape `getStopName`.
///
/// # Arguments
///
/// * `value` - one stop-name object.
/// * `stop` - stop number that was requested.
///
/// # Returns
///
/// `{source, stop, name}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::bus_stop_name;
/// let value = bus_stop_name(&serde_json::json!({"stopName": "Example Stop"}), "10001");
/// assert_eq!(value["name"], "Example Stop");
/// ```
pub fn bus_stop_name(value: &Value, stop: &str) -> Value {
    json!({
        "source": "stop_name",
        "stop": stop,
        "name": text(value, "stopName"),
    })
}

/// Shape `getBusLocationsData` (stops near a point, not vehicles).
///
/// # Arguments
///
/// * `value` - JSON array.
///
/// # Returns
///
/// `{source, stops}` including latitude, longitude, and distance.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::bus_stops_near;
/// let value = bus_stops_near(&serde_json::json!([{
///     "busstopnumber": "10001",
///     "busstopdescription": "Example Stop",
///     "latitude": "40.0",
///     "longitude": "-74.0",
///     "distance": "10",
///     "modetype": "BUS"
/// }]));
/// assert_eq!(value["source"], "nearby_stops");
/// assert_eq!(value["stops"][0]["latitude"], 40.0);
/// ```
pub fn bus_stops_near(value: &Value) -> Value {
    let rows = as_list(value)
        .into_iter()
        .map(|row| {
            json!({
                "stop": text(row, "busstopnumber"),
                "name": text(row, "busstopdescription"),
                "latitude": coord(&text(row, "latitude")),
                "longitude": coord(&text(row, "longitude")),
                "distance": number_or_text(&text(row, "distance")),
                "mode": empty_null(text(row, "modetype")),
            })
        })
        .collect::<Vec<_>>();
    json!({"source": "nearby_stops", "stops": rows})
}

/// Shape `getBusDV` or one `DVTrip` array from `getRouteTrips`.
///
/// # Arguments
///
/// * `value` - `DVResponse` object or an array of trips.
/// * `source` - `board` or `route_trips`.
///
/// # Returns
///
/// `{source, message, departures}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::bus_departures;
/// let value = bus_departures(&serde_json::json!({
///     "DVTrip": [{
///         "public_route": "99",
///         "header": "Example",
///         "departuretime": "10:00",
///         "internal_trip_number": "INT1",
///         "sched_dep_time": "6/22/2023 12:50:00 AM",
///         "timing_point_id": "TP1"
///     }]
/// }), "board");
/// assert_eq!(value["departures"][0]["internal_trip_number"], "INT1");
/// ```
pub fn bus_departures(value: &Value, source: &str) -> Value {
    let trips = if let Some(nested) = value.get("DVTrip") {
        as_list(nested)
    } else {
        as_list(value)
    };
    let rows = trips.into_iter().map(dv_trip).collect::<Vec<_>>();
    let message = value
        .get("message")
        .and_then(|message| message.get("message"))
        .map(|text| clip(&json_string(text), 400))
        .filter(|text| !text.is_empty());
    json!({
        "source": source,
        "message": message,
        "departures": rows,
    })
}

/// Shape `getTripStops`.
///
/// # Arguments
///
/// * `value` - JSON array of `tripItem`.
/// * `timing_point_id` - request field.
/// * `sched_dep_time` - request field.
/// * `internal_trip_number` - request field.
///
/// # Returns
///
/// The request keys plus `stops`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::bus_trip_stops;
/// let value = bus_trip_stops(&serde_json::json!([{
///     "TripNumber": "INT1",
///     "Description": "Example Stop",
///     "StopID": "10001",
///     "SchedDepTime": "10:00"
/// }]), "TP1", "6/22/2023 12:50:00 AM", "INT1");
/// assert_eq!(value["stops"][0]["stop"], "10001");
/// ```
pub fn bus_trip_stops(
    value: &Value,
    timing_point_id: &str,
    sched_dep_time: &str,
    internal_trip_number: &str,
) -> Value {
    let rows = as_list(value)
        .into_iter()
        .map(|row| {
            json!({
                "trip": empty_null(text(row, "TripNumber")),
                "time_point": empty_null(text(row, "TimePoint")),
                "description": empty_null(text(row, "Description")),
                "lane_gate": empty_null(text(row, "SchedLaneGate")),
                "manual_lane_gate": empty_null(text(row, "ManLaneGate")),
                "scheduled": empty_null(text(row, "SchedDepTime")),
                "approx": empty_null(text(row, "ApproxTime")),
                "stop": empty_null(text(row, "StopID")),
                "status": empty_null(text(row, "Status")),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "timing_point_id": timing_point_id,
        "sched_dep_time": sched_dep_time,
        "internal_trip_number": internal_trip_number,
        "stops": rows,
    })
}

/// Shape `getVehicleLocations`.
///
/// # Arguments
///
/// * `value` - JSON array.
/// * `latitude` - request latitude.
/// * `longitude` - request longitude.
/// * `radius` - request radius integer.
/// * `mode` - request mode.
///
/// # Returns
///
/// `{latitude, longitude, radius, mode, vehicles}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::bus_vehicles_near;
/// let value = bus_vehicles_near(&serde_json::json!([{
///     "VehicleID": "V1",
///     "VehicleRoute": "99",
///     "VehicleLat": "40.1",
///     "VehicleLong": "-74.1",
///     "VehicleSecondsLate": "15"
/// }]), "40.0", "-74.0", 1000, "BUS");
/// assert_eq!(value["vehicles"][0]["sec_late"], 15);
/// ```
pub fn bus_vehicles_near(
    value: &Value,
    latitude: &str,
    longitude: &str,
    radius: u32,
    mode: &str,
) -> Value {
    let rows = as_list(value)
        .into_iter()
        .map(|row| {
            json!({
                "id": text(row, "VehicleID"),
                "route": empty_null(text(row, "VehicleRoute")),
                "destination": empty_null(text(row, "VehicleDestination")),
                "latitude": coord(&text(row, "VehicleLat")),
                "longitude": coord(&text(row, "VehicleLong")),
                "distance_miles": number_or_text(&text(row, "VehicleDistanceMiles")),
                "passenger_load": empty_null(text(row, "VehiclePassengerLoad")),
                "sec_late": number_or_text(&text(row, "VehicleSecondsLate")),
                "internal_trip_number": empty_null(text(row, "VehicleInternalTripNumber")),
                "scheduled": empty_null(text(row, "VehicleScheduledDeparture")),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "latitude": latitude,
        "longitude": longitude,
        "radius": radius,
        "mode": mode,
        "vehicles": rows,
    })
}

/// Clip text to a character budget.
///
/// # Arguments
///
/// * `text` - input.
/// * `max_chars` - maximum characters. Zero yields an empty string.
///
/// # Returns
///
/// A string no longer than `max_chars` characters.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::shape::clip;
/// assert_eq!(clip("abcdef", 3), "abc");
/// ```
pub fn clip(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Shape one rail departure, keeping a limited number of stops.
///
/// # Arguments
///
/// * `row` - see the signature.
/// * `stop_limit` - see the signature.
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
fn departure_row(row: &Value, stop_limit: usize) -> Value {
    let stops = as_list(row.get("STOPS").unwrap_or(&Value::Null))
        .into_iter()
        .take(stop_limit)
        .map(stop_row)
        .collect::<Vec<_>>();
    json!({
        "train": text(row, "TRAIN_ID"),
        "destination": empty_null(text(row, "DESTINATION")),
        "track": empty_null(text(row, "TRACK")),
        "line": empty_null(text(row, "LINE")),
        "line_code": empty_null(first_nonempty(&[text(row, "LINECODE"), text(row, "LINEABBREVIATION")])),
        "status": empty_null(text(row, "STATUS")),
        "sec_late": number_or_text(&text(row, "SEC_LATE")),
        "scheduled": empty_null(text(row, "SCHED_DEP_DATE")),
        "latitude": coord(&text(row, "GPSLATITUDE")),
        "longitude": coord(&text(row, "GPSLONGITUDE")),
        "inline_message": empty_null(clip(&text(row, "INLINEMSG"), 200)),
        "stops": stops,
    })
}

/// Shape one rail stop row.
///
/// # Arguments
///
/// * `row` - see the signature.
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
fn stop_row(row: &Value) -> Value {
    json!({
        "station": text(row, "STATION_2CHAR"),
        "name": text(row, "STATIONNAME"),
        "time": empty_null(first_nonempty(&[text(row, "DEP_TIME"), text(row, "TIME")])),
        "departed": empty_null(text(row, "DEPARTED")),
        "status": empty_null(text(row, "STOP_STATUS")),
        "pickup": empty_null(text(row, "PICKUP")),
        "dropoff": empty_null(text(row, "DROPOFF")),
    })
}

/// Shape one bus departure-vision trip.
///
/// # Arguments
///
/// * `row` - see the signature.
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
fn dv_trip(row: &Value) -> Value {
    json!({
        "route": empty_null(text(row, "public_route")),
        "header": empty_null(text(row, "header")),
        "lane_gate": empty_null(text(row, "lanegate")),
        "departure_time": empty_null(text(row, "departuretime")),
        "status": empty_null(text(row, "departurestatus")),
        "remarks": empty_null(clip(&text(row, "remarks"), 200)),
        "internal_trip_number": empty_null(text(row, "internal_trip_number")),
        "sched_dep_time": empty_null(text(row, "sched_dep_time")),
        "timing_point_id": empty_null(text(row, "timing_point_id")),
        "passenger_load": empty_null(text(row, "passload")),
        "vehicle_id": empty_null(text(row, "vehicle_id")),
    })
}

/// Treat a JSON array as rows and a JSON object as a one-row list.
///
/// # Arguments
///
/// * `value` - see the signature.
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
fn as_list(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![value],
        _ => Vec::new(),
    }
}

/// Read one object field as trimmed text.
///
/// # Arguments
///
/// * `value` - see the signature.
/// * `key` - see the signature.
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
fn text(value: &Value, key: &str) -> String {
    json_string(value.get(key).unwrap_or(&Value::Null))
}

/// Render a JSON scalar as trimmed text.
///
/// # Arguments
///
/// * `value` - see the signature.
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
fn json_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

/// Turn an empty string into JSON null.
///
/// # Arguments
///
/// * `text` - see the signature.
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
fn empty_null(text: String) -> Value {
    if text.is_empty() {
        Value::Null
    } else {
        Value::String(text)
    }
}

/// Return the first non-empty string.
///
/// # Arguments
///
/// * `values` - see the signature.
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
fn first_nonempty(values: &[String]) -> String {
    values
        .iter()
        .find(|value| !value.is_empty())
        .cloned()
        .unwrap_or_default()
}

/// Parse an integer or decimal string, otherwise keep the text.
///
/// # Arguments
///
/// * `text` - see the signature.
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
fn number_or_text(text: &str) -> Value {
    if text.is_empty() {
        return Value::Null;
    }
    if let Ok(value) = text.parse::<i64>() {
        return json!(value);
    }
    if let Ok(value) = text.parse::<f64>() {
        return json!(value);
    }
    Value::String(text.to_string())
}

/// Parse a coordinate and round it to five decimals.
///
/// # Arguments
///
/// * `text` - see the signature.
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
fn coord(text: &str) -> Value {
    let Ok(value) = text.parse::<f64>() else {
        return Value::Null;
    };
    json!((value * 100_000.0).round() / 100_000.0)
}

/// Map Y/N style flags to a boolean.
///
/// # Arguments
///
/// * `text` - see the signature.
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
fn wheelchair(text: String) -> Value {
    match text.to_ascii_lowercase().as_str() {
        "y" | "yes" | "true" | "1" => Value::Bool(true),
        "n" | "no" | "false" | "0" => Value::Bool(false),
        "" => Value::Null,
        _ => Value::String(text),
    }
}
