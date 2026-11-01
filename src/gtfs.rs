//! Minimal static GTFS reader for the cached NJ Transit zips.
//!
//! Only stops, routes, trips, stop times, and the non-standard
//! `trips_internal.txt` file are kept. The full schedule database from
//! `gtfs-rt-mcp` is not linked: that crate keeps its parser private.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use serde_json::{json, Value};
use zip::ZipArchive;

use crate::error::AppError;
use crate::shape::page;

/// One stop from `stops.txt`.
///
/// # Examples
///
/// ```
/// use njt_mcp::gtfs::Stop;
/// let stop = Stop {
///     stop_id: "EX1".to_string(),
///     name: "Example Station".to_string(),
///     code: String::new(),
///     lat: Some(40.0),
///     lon: Some(-74.0),
///     location_type: String::new(),
///     parent_station: String::new(),
/// };
/// assert_eq!(stop.stop_id, "EX1");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    /// `stop_id`.
    pub stop_id: String,
    /// `stop_name`.
    pub name: String,
    /// `stop_code`, or empty.
    pub code: String,
    /// `stop_lat`, when it parses.
    pub lat: Option<f64>,
    /// `stop_lon`, when it parses.
    pub lon: Option<f64>,
    /// `location_type`, or empty.
    pub location_type: String,
    /// `parent_station`, or empty.
    pub parent_station: String,
}

/// One route from `routes.txt`.
///
/// # Examples
///
/// ```
/// use njt_mcp::gtfs::Route;
/// let route = Route {
///     route_id: "R1".to_string(),
///     short_name: "99".to_string(),
///     long_name: "Example Route".to_string(),
///     route_type: 2,
/// };
/// assert_eq!(route.route_type_name(), "rail");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// `route_id`.
    pub route_id: String,
    /// `route_short_name`.
    pub short_name: String,
    /// `route_long_name`.
    pub long_name: String,
    /// `route_type` integer. Unknown text becomes `0`.
    pub route_type: i32,
}

impl Route {
    /// GTFS route type name.
    ///
    /// # Returns
    ///
    /// A short English label, or `other` when the code is not in the common set.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::gtfs::Route;
    /// let route = Route { route_id: "R".into(), short_name: String::new(), long_name: String::new(), route_type: 3 };
    /// assert_eq!(route.route_type_name(), "bus");
    /// ```
    pub fn route_type_name(&self) -> &'static str {
        match self.route_type {
            0 => "tram",
            1 => "subway",
            2 => "rail",
            3 => "bus",
            4 => "ferry",
            5 => "cable_tram",
            6 => "aerial_lift",
            7 => "funicular",
            11 => "trolleybus",
            12 => "monorail",
            _ => "other",
        }
    }
}

/// One trip from `trips.txt`.
///
/// # Examples
///
/// ```
/// use njt_mcp::gtfs::Trip;
/// let trip = Trip {
///     trip_id: "T1".into(),
///     route_id: "R1".into(),
///     service_id: "WK".into(),
///     headsign: "Example Terminal".into(),
///     short_name: "9001".into(),
///     direction_id: Some(0),
/// };
/// assert_eq!(trip.short_name, "9001");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Trip {
    /// `trip_id`.
    pub trip_id: String,
    /// `route_id`.
    pub route_id: String,
    /// `service_id`.
    pub service_id: String,
    /// `trip_headsign`.
    pub headsign: String,
    /// `trip_short_name`.
    pub short_name: String,
    /// `direction_id`, when present.
    pub direction_id: Option<u32>,
}

/// One row of `stop_times.txt`.
///
/// # Examples
///
/// ```
/// use njt_mcp::gtfs::StopTime;
/// let row = StopTime { stop_id: "EX1".into(), arrival: "10:00:00".into(), departure: "10:00:00".into(), sequence: 1 };
/// assert_eq!(row.sequence, 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopTime {
    /// `stop_id`.
    pub stop_id: String,
    /// `arrival_time`, or empty.
    pub arrival: String,
    /// `departure_time`, or empty.
    pub departure: String,
    /// `stop_sequence`.
    pub sequence: u32,
}

/// Parsed subset of a GTFS zip.
///
/// # Examples
///
/// ```
/// use njt_mcp::fixtures::example_zip;
/// use njt_mcp::gtfs::parse_zip;
/// let set = parse_zip(&example_zip()).unwrap();
/// assert!(set.has_trips_internal);
/// assert_eq!(set.search_stops("example", 10, 0)["stops"][0]["stop_id"], "EX1");
/// ```
#[derive(Debug, Clone)]
pub struct GtfsSet {
    /// Stops in file order.
    pub stops: Vec<Stop>,
    /// Routes in file order.
    pub routes: Vec<Route>,
    /// Trips keyed by `trip_id`.
    pub trips: HashMap<String, Trip>,
    /// Stop times keyed by `trip_id`, sorted by sequence.
    pub stop_times: HashMap<String, Vec<StopTime>>,
    /// `internal_trip_number` values from `trips_internal.txt`, keyed by `trip_id`.
    pub internal_by_trip: HashMap<String, String>,
    /// `trip_id` values keyed by `internal_trip_number`.
    pub trip_by_internal: HashMap<String, String>,
    /// Whether `trips_internal.txt` was present.
    pub has_trips_internal: bool,
}

impl GtfsSet {
    /// Search stops by id, code, or name.
    ///
    /// # Arguments
    ///
    /// * `query` - case-insensitive substring. Empty returns the first page.
    /// * `limit` - page size.
    /// * `offset` - rows to skip.
    ///
    /// # Returns
    ///
    /// `{stops, total, next_offset}`. Exact id and code matches come first.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::fixtures::example_zip;
    /// use njt_mcp::gtfs::parse_zip;
    /// let set = parse_zip(&example_zip()).unwrap();
    /// assert_eq!(set.search_stops("EX1", 5, 0)["stops"][0]["name"], "Example Station");
    /// ```
    pub fn search_stops(&self, query: &str, limit: usize, offset: usize) -> Value {
        let mut ranked = self
            .stops
            .iter()
            .filter_map(|stop| score_stop(stop, query).map(|score| (score, stop)))
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.stop_id.cmp(&right.1.stop_id))
        });
        let rows = ranked
            .into_iter()
            .map(|(_, stop)| {
                json!({
                    "stop_id": stop.stop_id,
                    "name": stop.name,
                    "code": empty(&stop.code),
                    "lat": stop.lat,
                    "lon": stop.lon,
                    "location_type": empty(&stop.location_type),
                    "parent_station": empty(&stop.parent_station),
                })
            })
            .collect();
        let paged = page(rows, offset, limit);
        json!({"stops": paged.rows, "total": paged.total, "next_offset": paged.next_offset})
    }

    /// Search routes by id, short name, or long name.
    ///
    /// # Arguments
    ///
    /// * `query` - case-insensitive substring. Empty returns the first page.
    /// * `limit` - page size.
    /// * `offset` - rows to skip.
    ///
    /// # Returns
    ///
    /// `{routes, total, next_offset}`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::fixtures::example_zip;
    /// use njt_mcp::gtfs::parse_zip;
    /// let set = parse_zip(&example_zip()).unwrap();
    /// assert_eq!(set.search_routes("99", 5, 0)["routes"][0]["route_type_name"], "rail");
    /// ```
    pub fn search_routes(&self, query: &str, limit: usize, offset: usize) -> Value {
        let query_l = query.trim().to_ascii_lowercase();
        let mut rows = self
            .routes
            .iter()
            .filter(|route| {
                query_l.is_empty()
                    || route.route_id.to_ascii_lowercase().contains(&query_l)
                    || route.short_name.to_ascii_lowercase().contains(&query_l)
                    || route.long_name.to_ascii_lowercase().contains(&query_l)
            })
            .map(|route| {
                json!({
                    "route_id": route.route_id,
                    "short_name": empty(&route.short_name),
                    "long_name": empty(&route.long_name),
                    "route_type": route.route_type,
                    "route_type_name": route.route_type_name(),
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            left["route_id"]
                .as_str()
                .unwrap_or("")
                .cmp(right["route_id"].as_str().unwrap_or(""))
        });
        let paged = page(rows, offset, limit);
        json!({"routes": paged.rows, "total": paged.total, "next_offset": paged.next_offset})
    }

    /// Look up one trip by `trip_id`, `trip_short_name`, or internal trip number.
    ///
    /// # Arguments
    ///
    /// * `id` - identifier to match.
    /// * `limit` - stop rows to return.
    /// * `offset` - stop rows to skip.
    ///
    /// # Returns
    ///
    /// Trip fields plus a page of stop times. `internal_trip_number` is set when
    /// `trips_internal.txt` had a row.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when nothing matches.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::fixtures::example_zip;
    /// use njt_mcp::gtfs::parse_zip;
    /// let set = parse_zip(&example_zip()).unwrap();
    /// let trip = set.trip("9001", 10, 0).unwrap();
    /// assert_eq!(trip["trip_id"], "T1");
    /// assert_eq!(trip["internal_trip_number"], "INT1");
    /// assert_eq!(trip["stops"][0]["stop_id"], "EX1");
    /// ```
    pub fn trip(&self, id: &str, limit: usize, offset: usize) -> Result<Value, AppError> {
        let trip = self.find_trip(id).ok_or_else(|| {
            AppError::new(format!(
                "no GTFS trip matches {id:?}. Try gtfs_routes or a trip_id from the feed."
            ))
        })?;
        let times = self
            .stop_times
            .get(&trip.trip_id)
            .cloned()
            .unwrap_or_default();
        let rows = times
            .iter()
            .map(|row| {
                let name = self
                    .stops
                    .iter()
                    .find(|stop| stop.stop_id == row.stop_id)
                    .map(|stop| stop.name.clone());
                json!({
                    "seq": row.sequence,
                    "stop_id": row.stop_id,
                    "name": name,
                    "arrival": empty(&row.arrival),
                    "departure": empty(&row.departure),
                })
            })
            .collect();
        let paged = page(rows, offset, limit);
        Ok(json!({
            "trip_id": trip.trip_id,
            "route_id": trip.route_id,
            "service_id": empty(&trip.service_id),
            "headsign": empty(&trip.headsign),
            "short_name": empty(&trip.short_name),
            "direction_id": trip.direction_id,
            "internal_trip_number": self.internal_by_trip.get(&trip.trip_id),
            "stops": paged.rows,
            "total": paged.total,
            "next_offset": paged.next_offset,
        }))
    }

    /// Resolve a trip by id, short name, or internal trip number.
    ///
    /// # Arguments
    ///
    /// * `id` - see the signature.
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
    fn find_trip(&self, id: &str) -> Option<&Trip> {
        let trimmed = id.trim();
        if let Some(trip) = self.trips.get(trimmed) {
            return Some(trip);
        }
        if let Some(trip_id) = self.trip_by_internal.get(trimmed) {
            if let Some(trip) = self.trips.get(trip_id) {
                return Some(trip);
            }
        }
        let lower = trimmed.to_ascii_lowercase();
        self.trips.values().find(|trip| {
            trip.trip_id.eq_ignore_ascii_case(trimmed)
                || trip.short_name.eq_ignore_ascii_case(trimmed)
                || (!lower.is_empty() && trip.headsign.to_ascii_lowercase() == lower)
        })
    }
}

/// Parse a GTFS zip from memory.
///
/// # Arguments
///
/// * `bytes` - zip file bytes. They must start as a real zip.
///
/// # Returns
///
/// A [`GtfsSet`]. `stops.txt`, `routes.txt`, and `trips.txt` are required.
///
/// # Errors
///
/// Returns [`AppError`] when the zip or a required file cannot be read, or when
/// `stop_times.txt` has more than two million rows.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::fixtures::example_zip;
/// use njt_mcp::gtfs::parse_zip;
/// let set = parse_zip(&example_zip()).unwrap();
/// assert_eq!(set.routes.len(), 1);
/// ```
pub fn parse_zip(bytes: &[u8]) -> Result<GtfsSet, AppError> {
    if bytes.first() != Some(&b'P') || bytes.get(1) != Some(&b'K') {
        return Err(AppError::new("GTFS response was not a zip"));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| AppError::new(format!("zip: {error}")))?;
    let stops = read_csv(&mut archive, "stops.txt")?;
    let routes = read_csv(&mut archive, "routes.txt")?;
    let trips = read_csv(&mut archive, "trips.txt")?;
    let stop_times = read_csv_optional(&mut archive, "stop_times.txt")?;
    let internal = read_csv_optional(&mut archive, "trips_internal.txt")?;
    let mut set = GtfsSet {
        stops: stops.iter().filter_map(stop_from).collect(),
        routes: routes.iter().filter_map(route_from).collect(),
        trips: HashMap::new(),
        stop_times: HashMap::new(),
        internal_by_trip: HashMap::new(),
        trip_by_internal: HashMap::new(),
        has_trips_internal: internal.is_some(),
    };
    for row in &trips {
        if let Some(trip) = trip_from(row) {
            set.trips.insert(trip.trip_id.clone(), trip);
        }
    }
    if let Some(rows) = stop_times {
        if rows.len() > 2_000_000 {
            return Err(AppError::new(
                "stop_times.txt is larger than this server will load",
            ));
        }
        for row in rows {
            let Some(trip_id) = row.get("trip_id").cloned() else {
                continue;
            };
            let sequence = row
                .get("stop_sequence")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            set.stop_times.entry(trip_id).or_default().push(StopTime {
                stop_id: row.get("stop_id").cloned().unwrap_or_default(),
                arrival: row.get("arrival_time").cloned().unwrap_or_default(),
                departure: row.get("departure_time").cloned().unwrap_or_default(),
                sequence,
            });
        }
        for times in set.stop_times.values_mut() {
            times.sort_by_key(|row| row.sequence);
        }
    }
    if let Some(rows) = internal {
        for row in rows {
            let trip_id = row
                .get("trip_id")
                .cloned()
                .filter(|value| !value.is_empty());
            let internal_number = ["internal_trip_number", "internal_trip", "trip_number"]
                .iter()
                .find_map(|key| row.get(*key).cloned())
                .filter(|value| !value.is_empty());
            if let (Some(trip_id), Some(number)) = (trip_id, internal_number) {
                set.trip_by_internal.insert(number.clone(), trip_id.clone());
                set.internal_by_trip.insert(trip_id, number);
            }
        }
    }
    Ok(set)
}

/// Rank a stop against a search string. Lower scores sort first.
///
/// # Arguments
///
/// * `stop` - see the signature.
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
fn score_stop(stop: &Stop, query: &str) -> Option<u8> {
    let query = query.trim();
    if query.is_empty() {
        return Some(3);
    }
    let q = query.to_ascii_lowercase();
    if stop.stop_id.eq_ignore_ascii_case(query) {
        Some(0)
    } else if !stop.code.is_empty() && stop.code.eq_ignore_ascii_case(query) {
        Some(1)
    } else if stop.name.to_ascii_lowercase().contains(&q)
        || stop.stop_id.to_ascii_lowercase().contains(&q)
    {
        Some(2)
    } else {
        None
    }
}

/// Build a stop from one CSV row, or skip a row with no stop_id.
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
fn stop_from(row: &HashMap<String, String>) -> Option<Stop> {
    let stop_id = row.get("stop_id")?.trim().to_string();
    if stop_id.is_empty() {
        return None;
    }
    Some(Stop {
        stop_id,
        name: row.get("stop_name").cloned().unwrap_or_default(),
        code: row.get("stop_code").cloned().unwrap_or_default(),
        lat: row.get("stop_lat").and_then(|value| value.parse().ok()),
        lon: row.get("stop_lon").and_then(|value| value.parse().ok()),
        location_type: row.get("location_type").cloned().unwrap_or_default(),
        parent_station: row.get("parent_station").cloned().unwrap_or_default(),
    })
}

/// Build a route from one CSV row, or skip a row with no route_id.
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
fn route_from(row: &HashMap<String, String>) -> Option<Route> {
    let route_id = row.get("route_id")?.trim().to_string();
    if route_id.is_empty() {
        return None;
    }
    Some(Route {
        route_id,
        short_name: row.get("route_short_name").cloned().unwrap_or_default(),
        long_name: row.get("route_long_name").cloned().unwrap_or_default(),
        route_type: row
            .get("route_type")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
    })
}

/// Build a trip from one CSV row, or skip a row with no trip_id.
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
fn trip_from(row: &HashMap<String, String>) -> Option<Trip> {
    let trip_id = row.get("trip_id")?.trim().to_string();
    if trip_id.is_empty() {
        return None;
    }
    Some(Trip {
        trip_id,
        route_id: row.get("route_id").cloned().unwrap_or_default(),
        service_id: row.get("service_id").cloned().unwrap_or_default(),
        headsign: row.get("trip_headsign").cloned().unwrap_or_default(),
        short_name: row.get("trip_short_name").cloned().unwrap_or_default(),
        direction_id: row.get("direction_id").and_then(|value| value.parse().ok()),
    })
}

/// Read a required CSV member from the zip.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn read_csv<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Vec<HashMap<String, String>>, AppError> {
    read_csv_optional(archive, name)?
        .ok_or_else(|| AppError::new(format!("GTFS zip is missing {name}")))
}

/// Read a CSV member, or return None when the zip does not contain it.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn read_csv_optional<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<HashMap<String, String>>>, AppError> {
    let mut file = match archive.by_name(name) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|error| AppError::new(format!("{name}: {error}")))?;
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|error| AppError::new(format!("{name}: {error}")))?
        .iter()
        .map(|header| header.trim().to_string())
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| AppError::new(format!("{name}: {error}")))?;
        let mut row = HashMap::new();
        for (index, header) in headers.iter().enumerate() {
            if header.is_empty() {
                continue;
            }
            if let Some(value) = record.get(index) {
                row.insert(header.clone(), value.trim().to_string());
            }
        }
        rows.push(row);
    }
    Ok(Some(rows))
}

/// Turn a blank string into JSON null.
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
fn empty(text: &str) -> Value {
    if text.is_empty() {
        Value::Null
    } else {
        Value::String(text.to_string())
    }
}
