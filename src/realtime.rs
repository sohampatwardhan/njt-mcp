//! Decode GTFS-Realtime protobufs into compact JSON.
//!
//! The bytes come from `getTripUpdates`, `getVehiclePositions`, and `getAlerts`
//! on the rail `GTFSRT` API or the bus `GTFSG2` API. Both products return the
//! standard protobuf, not JSON.

use prost::Message;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::gtfs_rt::trip_descriptor::ScheduleRelationship as TripRel;
use crate::gtfs_rt::trip_update::stop_time_update::ScheduleRelationship as StopRel;
use crate::gtfs_rt::vehicle_position::VehicleStopStatus;
use crate::gtfs_rt::{alert, FeedMessage, TranslatedString};
use crate::shape::{clip, page};

/// Query shared by the three GTFS-Realtime tools.
///
/// # Examples
///
/// ```
/// use njt_mcp::realtime::RtQuery;
/// use njt_mcp::Transit;
/// let query = RtQuery {
///     transit: Transit::Rail,
///     route: Some("R1".into()),
///     trip: None,
///     stop: None,
///     text: None,
///     limit: 20,
///     offset: 0,
///     stop_limit: 8,
///     max_chars: 400,
/// };
/// assert_eq!(query.transit, Transit::Rail);
/// ```
#[derive(Debug, Clone)]
pub struct RtQuery {
    /// Rail or bus feed.
    pub transit: crate::api::Transit,
    /// `route_id` filter. Empty skips the filter.
    pub route: Option<String>,
    /// `trip_id` filter.
    pub trip: Option<String>,
    /// `stop_id` filter.
    pub stop: Option<String>,
    /// Case-insensitive alert text filter.
    pub text: Option<String>,
    /// Page size.
    pub limit: usize,
    /// Rows to skip.
    pub offset: usize,
    /// Stop-time updates kept per trip.
    pub stop_limit: usize,
    /// Alert text character budget.
    pub max_chars: usize,
}

/// Gunzip a payload when it starts with the gzip magic, otherwise return it.
///
/// # Arguments
///
/// * `bytes` - response body.
///
/// # Returns
///
/// Inflated bytes, or a copy of `bytes`.
///
/// # Errors
///
/// Returns [`AppError`] when the gzip header is present but the body is corrupt.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::realtime::gunzip_if_needed;
/// assert_eq!(gunzip_if_needed(b"abc").unwrap(), b"abc");
/// ```
pub fn gunzip_if_needed(bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut decoder = flate2::read::GzDecoder::new(bytes);
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut out)
            .map_err(|error| AppError::new(format!("gzip: {error}")))?;
        Ok(out)
    } else {
        Ok(bytes.to_vec())
    }
}

/// Decode one `FeedMessage`.
///
/// # Arguments
///
/// * `bytes` - protobuf or gzip-wrapped protobuf.
///
/// # Returns
///
/// The decoded message.
///
/// # Errors
///
/// Returns [`AppError`] when the bytes are empty, look like JSON or `Null`, or
/// are not a `FeedMessage`.
///
/// # Panics
///
/// This function does not panic. The gzip magic check indexes only after a length test;
/// if that ever tripped, the function would panic, so the length test stays first.
///
/// # Examples
///
/// ```
/// use njt_mcp::fixtures::example_feed_bytes;
/// use njt_mcp::realtime::decode_feed;
/// let feed = decode_feed(&example_feed_bytes()).unwrap();
/// assert_eq!(feed.entity.len(), 3);
/// ```
pub fn decode_feed(bytes: &[u8]) -> Result<FeedMessage, AppError> {
    let raw = gunzip_if_needed(bytes)?;
    if raw.is_empty() {
        return Err(AppError::new("GTFS-Realtime payload is empty"));
    }
    let first = raw.first().copied().unwrap_or(0);
    if first == b'{' || first == b'[' || first == b'N' || first == b'n' {
        return Err(AppError::new(
            "GTFS-Realtime payload was JSON or Null, not a protobuf",
        ));
    }
    FeedMessage::decode(raw.as_slice()).map_err(|error| {
        AppError::new(format!("protobuf: {error}"))
            .hint("the bytes are not a GTFS-Realtime FeedMessage")
    })
}

/// Project trip updates.
///
/// # Arguments
///
/// * `feed` - decoded message.
/// * `query` - filters and page size.
///
/// # Returns
///
/// `{service, timestamp, trip_updates, total, next_offset}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::fixtures::example_feed;
/// use njt_mcp::realtime::{project_trip_updates, RtQuery};
/// use njt_mcp::Transit;
/// let query = RtQuery {
///     transit: Transit::Rail,
///     route: Some("R1".into()),
///     trip: None,
///     stop: Some("EX1".into()),
///     text: None,
///     limit: 10,
///     offset: 0,
///     stop_limit: 4,
///     max_chars: 200,
/// };
/// let value = project_trip_updates(&example_feed(), &query);
/// assert_eq!(value["trip_updates"][0]["trip_id"], "T1");
/// assert_eq!(value["trip_updates"][0]["stops"][0]["arrival_delay"], 60);
/// ```
pub fn project_trip_updates(feed: &FeedMessage, query: &RtQuery) -> Value {
    let rows = feed
        .entity
        .iter()
        .filter_map(|entity| entity.trip_update.as_ref().map(|update| (entity, update)))
        .filter(|(_, update)| trip_matches(&update.trip, query))
        .filter(|(_, update)| stop_matches_update(update, query))
        .map(|(entity, update)| {
            let stops = update
                .stop_time_update
                .iter()
                .take(query.stop_limit)
                .map(|stop| {
                    json!({
                        "stop_id": stop.stop_id,
                        "stop_sequence": stop.stop_sequence,
                        "arrival_delay": stop.arrival.as_ref().and_then(|event| event.delay),
                        "arrival_time": stop.arrival.as_ref().and_then(|event| event.time),
                        "departure_delay": stop.departure.as_ref().and_then(|event| event.delay),
                        "departure_time": stop.departure.as_ref().and_then(|event| event.time),
                        "schedule_relationship": enum_name::<StopRel>(stop.schedule_relationship),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "id": entity.id,
                "trip_id": update.trip.trip_id,
                "route_id": update.trip.route_id,
                "start_date": update.trip.start_date,
                "start_time": update.trip.start_time,
                "schedule_relationship": enum_name::<TripRel>(update.trip.schedule_relationship),
                "delay_sec": update.delay,
                "timestamp": update.timestamp,
                "stops": stops,
            })
        })
        .collect();
    finish("trip_updates", feed, query, rows)
}

/// Project vehicle positions.
///
/// # Arguments
///
/// * `feed` - decoded message.
/// * `query` - filters and page size. `text` is ignored.
///
/// # Returns
///
/// `{service, timestamp, vehicles, total, next_offset}`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::fixtures::example_feed;
/// use njt_mcp::realtime::{project_vehicles, RtQuery};
/// use njt_mcp::Transit;
/// let query = RtQuery {
///     transit: Transit::Bus,
///     route: None,
///     trip: Some("T1".into()),
///     stop: None,
///     text: None,
///     limit: 10,
///     offset: 0,
///     stop_limit: 4,
///     max_chars: 200,
/// };
/// let value = project_vehicles(&example_feed(), &query);
/// assert_eq!(value["vehicles"][0]["vehicle_id"], "V1");
/// assert_eq!(value["service"], "bus");
/// ```
pub fn project_vehicles(feed: &FeedMessage, query: &RtQuery) -> Value {
    let rows = feed
        .entity
        .iter()
        .filter_map(|entity| entity.vehicle.as_ref().map(|vehicle| (entity, vehicle)))
        .filter(|(_, vehicle)| {
            vehicle
                .trip
                .as_ref()
                .map(|trip| trip_matches(trip, query))
                .unwrap_or_else(|| query.route.is_none() && query.trip.is_none())
        })
        .filter(|(_, vehicle)| match query.stop.as_deref().map(str::trim) {
            None | Some("") => true,
            Some(stop) => vehicle
                .stop_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(stop)),
        })
        .map(|(entity, vehicle)| {
            json!({
                "id": entity.id,
                "trip_id": vehicle.trip.as_ref().and_then(|trip| trip.trip_id.clone()),
                "route_id": vehicle.trip.as_ref().and_then(|trip| trip.route_id.clone()),
                "vehicle_id": vehicle.vehicle.as_ref().and_then(|item| item.id.clone()),
                "label": vehicle.vehicle.as_ref().and_then(|item| item.label.clone()),
                "latitude": vehicle.position.as_ref().map(|position| round5(position.latitude)),
                "longitude": vehicle.position.as_ref().map(|position| round5(position.longitude)),
                "bearing": vehicle.position.as_ref().and_then(|position| position.bearing),
                "speed_mps": vehicle.position.as_ref().and_then(|position| position.speed),
                "stop_id": vehicle.stop_id,
                "stop_sequence": vehicle.current_stop_sequence,
                "status": enum_name::<VehicleStopStatus>(vehicle.current_status),
                "timestamp": vehicle.timestamp,
            })
        })
        .collect();
    finish("vehicles", feed, query, rows)
}

/// Project service alerts.
///
/// # Arguments
///
/// * `feed` - decoded message.
/// * `query` - route, stop, and text filters.
///
/// # Returns
///
/// `{service, timestamp, alerts, total, next_offset}`.
///
/// Periods include the deprecated `active_period` plus `communication_period`
/// and `impact_period`.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::fixtures::example_feed;
/// use njt_mcp::realtime::{project_alerts, RtQuery};
/// use njt_mcp::Transit;
/// let query = RtQuery {
///     transit: Transit::Rail,
///     route: Some("R1".into()),
///     trip: None,
///     stop: None,
///     text: Some("example".into()),
///     limit: 10,
///     offset: 0,
///     stop_limit: 4,
///     max_chars: 200,
/// };
/// let value = project_alerts(&example_feed(), &query);
/// assert_eq!(value["alerts"][0]["header"], "Example alert");
/// assert_eq!(value["alerts"][0]["cause"], "MAINTENANCE");
/// ```
#[allow(deprecated)]
pub fn project_alerts(feed: &FeedMessage, query: &RtQuery) -> Value {
    let rows = feed
        .entity
        .iter()
        .filter_map(|entity| entity.alert.as_ref().map(|alert| (entity, alert)))
        .filter(|(_, alert)| alert_matches(alert, query))
        .map(|(entity, alert)| {
            let (header, _) = translated(alert.header_text.as_ref(), query.max_chars);
            let (description, _) = translated(alert.description_text.as_ref(), query.max_chars);
            json!({
                "id": entity.id,
                "cause": enum_name::<alert::Cause>(alert.cause),
                "effect": enum_name::<alert::Effect>(alert.effect),
                "severity": enum_name::<alert::SeverityLevel>(alert.severity_level),
                "header": header,
                "description": description,
                "routes": alert.informed_entity.iter().filter_map(|item| item.route_id.clone()).collect::<Vec<_>>(),
                "stops": alert.informed_entity.iter().filter_map(|item| item.stop_id.clone()).collect::<Vec<_>>(),
                "active_period": periods(&alert.active_period),
                "communication_period": periods(&alert.communication_period),
                "impact_period": periods(&alert.impact_period),
            })
        })
        .collect();
    finish("alerts", feed, query, rows)
}

/// Serialize a tool result, or the error JSON the client should see.
///
/// # Arguments
///
/// * `key` - see the signature.
/// * `feed` - see the signature.
/// * `query` - see the signature.
/// * `rows` - see the signature.
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
fn finish(key: &str, feed: &FeedMessage, query: &RtQuery, rows: Vec<Value>) -> Value {
    let paged = page(rows, query.offset, query.limit);
    json!({
        "service": query.transit.as_str(),
        "timestamp": feed.header.timestamp,
        key: paged.rows,
        "total": paged.total,
        "next_offset": paged.next_offset,
    })
}

/// Whether a trip descriptor satisfies the route and trip filters.
///
/// # Arguments
///
/// * `trip` - see the signature.
/// * `query` - see the signature.
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
fn trip_matches(trip: &crate::gtfs_rt::TripDescriptor, query: &RtQuery) -> bool {
    let route_ok = match query.route.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(route) => trip
            .route_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(route)),
    };
    let trip_ok = match query.trip.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(wanted) => trip
            .trip_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(wanted)),
    };
    route_ok && trip_ok
}

/// Whether any stop-time update mentions the requested stop.
///
/// # Arguments
///
/// * `update` - see the signature.
/// * `query` - see the signature.
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
fn stop_matches_update(update: &crate::gtfs_rt::TripUpdate, query: &RtQuery) -> bool {
    match query.stop.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(stop) => update.stop_time_update.iter().any(|row| {
            row.stop_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(stop))
        }),
    }
}

/// Whether an alert satisfies the route, stop, and text filters.
///
/// # Arguments
///
/// * `alert` - see the signature.
/// * `query` - see the signature.
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
fn alert_matches(alert: &crate::gtfs_rt::Alert, query: &RtQuery) -> bool {
    let route_ok = match query.route.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(route) => alert.informed_entity.iter().any(|item| {
            item.route_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(route))
        }),
    };
    let stop_ok = match query.stop.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(stop) => alert.informed_entity.iter().any(|item| {
            item.stop_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(stop))
        }),
    };
    let text_ok = match query.text.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(needle) => {
            let (header, _) = translated(alert.header_text.as_ref(), 2_000);
            let (description, _) = translated(alert.description_text.as_ref(), 2_000);
            let haystack = format!(
                "{} {}",
                header.unwrap_or_default(),
                description.unwrap_or_default()
            )
            .to_ascii_lowercase();
            haystack.contains(&needle.to_ascii_lowercase())
        }
    };
    route_ok && stop_ok && text_ok
}

/// Render GTFS-Realtime time ranges as JSON.
///
/// # Arguments
///
/// * `ranges` - see the signature.
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
fn periods(ranges: &[crate::gtfs_rt::TimeRange]) -> Vec<Value> {
    ranges
        .iter()
        .map(|range| json!({"start": range.start, "end": range.end}))
        .collect()
}

/// Pick an English translation and clip it.
///
/// # Arguments
///
/// * `text` - see the signature.
/// * `max_chars` - see the signature.
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
fn translated(text: Option<&TranslatedString>, max_chars: usize) -> (Option<String>, bool) {
    let Some(text) = text else {
        return (None, false);
    };
    let chosen = text
        .translation
        .iter()
        .find(|item| {
            item.language.as_deref().is_some_and(|lang| {
                lang.eq_ignore_ascii_case("en") || lang.to_ascii_lowercase().starts_with("en-")
            })
        })
        .or_else(|| text.translation.iter().find(|item| item.language.is_none()))
        .or(text.translation.first());
    let Some(chosen) = chosen else {
        return (None, false);
    };
    let clipped = clip(&chosen.text, max_chars);
    let truncated = clipped.chars().count() < chosen.text.chars().count();
    (Some(clipped), truncated)
}

/// Map a protobuf enum number to its proto name.
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
fn enum_name<E>(value: Option<i32>) -> Option<String>
where
    E: TryFrom<i32> + Copy + ProtoName,
{
    value
        .and_then(|number| E::try_from(number).ok())
        .map(|item| item.proto_name().to_string())
}

/// Protobuf enum that can print its schema name.
///
/// # Examples
///
/// Constructed inside this module.
trait ProtoName {
    /// Return the protobuf enum name.
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
    fn proto_name(self) -> &'static str;
}

impl ProtoName for TripRel {
    /// Return the protobuf enum name.
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
    fn proto_name(self) -> &'static str {
        self.as_str_name()
    }
}

impl ProtoName for StopRel {
    /// Return the protobuf enum name.
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
    fn proto_name(self) -> &'static str {
        self.as_str_name()
    }
}

impl ProtoName for VehicleStopStatus {
    /// Return the protobuf enum name.
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
    fn proto_name(self) -> &'static str {
        self.as_str_name()
    }
}

impl ProtoName for alert::Cause {
    /// Return the protobuf enum name.
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
    fn proto_name(self) -> &'static str {
        self.as_str_name()
    }
}

impl ProtoName for alert::Effect {
    /// Return the protobuf enum name.
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
    fn proto_name(self) -> &'static str {
        self.as_str_name()
    }
}

impl ProtoName for alert::SeverityLevel {
    /// Return the protobuf enum name.
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
    fn proto_name(self) -> &'static str {
        self.as_str_name()
    }
}

/// Round a coordinate to five decimal places.
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
fn round5(value: f32) -> f64 {
    let value = f64::from(value);
    (value * 100_000.0).round() / 100_000.0
}
