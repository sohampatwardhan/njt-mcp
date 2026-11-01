//! Tool implementations shared by stdio and HTTP.
//!
//! Each method returns compact JSON or an [`AppError`]. Upstream calls go
//! through one token-aware fetch path so daily-limit and invalid-token handling
//! stays in one place.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use crate::api::{Api, Transit};
use crate::auth::Tokens;
use crate::client::{max_bytes_for, Client};
use crate::config::Config;
use crate::error::{redact_error, AppError, ErrorKind};
use crate::gtfs::{self, GtfsSet};
use crate::limits::REFERENCE_TTL_SECS;
use crate::realtime::{self, RtQuery};
use crate::shape::{self, station_key};
use crate::store::Store;
use crate::upstream::{self, Classified};

/// How a successful response should be cached and counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallClass {
    /// Station lists, routes, and other slow-changing data.
    Reference,
    /// Departures, vehicles, and GTFS-Realtime.
    Live,
    /// `getStationSchedule`, counted against the daily schedule cap by the caller.
    Schedule,
    /// GTFS zip. Cached on disk by the caller.
    Zip,
}

/// One memory-cache value and the instant it was stored.
///
/// # Examples
///
/// Constructed inside this module.
struct MemEntry {
    /// Instant the body was stored.
    at: Instant,
    /// Cached response body.
    bytes: Vec<u8>,
}

/// Parsed GTFS sets kept for this process.
///
/// # Examples
///
/// Constructed inside this module.
struct GtfsMemory {
    /// Parsed rail GTFS after the first successful load.
    rail: Option<Arc<GtfsSet>>,
    /// Parsed bus GTFS after the first successful load.
    bus: Option<Arc<GtfsSet>>,
}

/// Arguments for `bus_stops`.
///
/// Set `latitude` and `longitude` to search stops near a point. Set `stop`
/// alone to resolve one stop number. Otherwise set `route` and `direction`.
///
/// # Examples
///
/// ```
/// use njt_mcp::service::BusStopQuery;
/// let query = BusStopQuery {
///     route: Some("99".into()),
///     direction: Some("To Example".into()),
///     name_contains: None,
///     stop: None,
///     latitude: None,
///     longitude: None,
///     radius: None,
///     mode: Some("BUS".into()),
///     limit: 20,
///     offset: 0,
/// };
/// assert_eq!(query.route.as_deref(), Some("99"));
/// ```
#[derive(Debug, Clone)]
pub struct BusStopQuery {
    /// Route id for `getStops` or `getBusLocationsData`.
    pub route: Option<String>,
    /// Direction name from `bus_directions`.
    pub direction: Option<String>,
    /// Optional `namecontains` filter.
    pub name_contains: Option<String>,
    /// Stop number for `getStopName` when route is omitted.
    pub stop: Option<String>,
    /// Latitude for a nearby-stop search.
    pub latitude: Option<String>,
    /// Longitude for a nearby-stop search.
    pub longitude: Option<String>,
    /// Radius for a nearby-stop search. The API guide describes this as feet.
    /// Swagger types it as an integer and does not repeat the unit.
    pub radius: Option<u32>,
    /// `BUS`, `NLR`, `HBLR`, `RL`, or `ALL`.
    pub mode: Option<String>,
    /// Page size.
    pub limit: usize,
    /// Rows to skip.
    pub offset: usize,
}

/// Arguments for `bus_departures`.
///
/// Provide `location` and `route` for `getRouteTrips`. Otherwise provide `stop`
/// for `getBusDV`.
///
/// # Examples
///
/// ```
/// use njt_mcp::service::BusDepartureQuery;
/// let query = BusDepartureQuery {
///     stop: Some("10001".into()),
///     direction: None,
///     route: Some("99".into()),
///     ip: None,
///     minutes_requested: None,
///     max_variants: None,
///     sort_mode: None,
///     location: None,
/// };
/// assert!(query.location.is_none());
/// ```
#[derive(Debug, Clone, Default)]
pub struct BusDepartureQuery {
    /// Stop id for the departure board.
    pub stop: Option<String>,
    /// Direction filter.
    pub direction: Option<String>,
    /// Route filter.
    pub route: Option<String>,
    /// Client address the departure-board method accepts as `IP`.
    pub ip: Option<String>,
    /// Optional swagger field `minutesRequested`.
    pub minutes_requested: Option<u32>,
    /// Optional swagger field `maxVariants`.
    pub max_variants: Option<u32>,
    /// Optional swagger field `sortMode`.
    pub sort_mode: Option<String>,
    /// `bus_terminal_code` or stop number for `getRouteTrips`.
    pub location: Option<String>,
}

/// NJ Transit tools backed by one token manager and one disk cache.
///
/// # Examples
///
/// ```no_run
/// use njt_mcp::{Config, Service};
/// # fn demo() -> Result<(), njt_mcp::AppError> {
/// let config = Config::parse([
///     ("NJT_TOKEN", "replace-me"),
///     ("NJT_CACHE_DIR", "/tmp/njt-mcp-doctest"),
/// ]).expect("config");
/// let service = Service::new(config)?;
/// let _ = service;
/// # Ok(())
/// # }
/// ```
pub struct Service {
    /// Parsed environment.
    config: Config,
    /// HTTP client. It does not store credentials.
    client: Client,
    /// Disk cache shared with the token manager.
    store: Arc<Store>,
    /// Per-API token manager.
    tokens: Tokens,
    /// Short-lived response bodies.
    mem: Mutex<HashMap<String, MemEntry>>,
    /// Parsed static GTFS kept in memory.
    gtfs: Mutex<GtfsMemory>,
    /// Serializes schedule fetches so the daily cap cannot be raced.
    schedule_gate: AsyncMutex<()>,
    /// One gate for the rail zip and one for the bus zip.
    gtfs_gates: [AsyncMutex<()>; 2],
    /// Latest scrubbed error message per API.
    last_error: Mutex<HashMap<Api, String>>,
}

impl Service {
    /// Open the cache directory and build the HTTP client. Does not call NJ Transit.
    ///
    /// # Arguments
    ///
    /// * `config` - parsed environment.
    ///
    /// # Returns
    ///
    /// A service that is safe to share across tasks.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the cache directory or HTTP client cannot be created.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Config, Service};
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-svc-{}", std::process::id()));
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", dir.to_str().unwrap()),
    /// ])
    /// .unwrap();
    /// assert!(Service::new(config).is_ok());
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn new(config: Config) -> Result<Self, AppError> {
        let client = Client::new(&config)?;
        let store = Arc::new(Store::open(config.cache_dir())?);
        let tokens = Tokens::new(
            config.credentials().clone(),
            client.clone(),
            Arc::clone(&store),
        );
        Ok(Self {
            config,
            client,
            store,
            tokens,
            mem: Mutex::new(HashMap::new()),
            gtfs: Mutex::new(GtfsMemory {
                rail: None,
                bus: None,
            }),
            schedule_gate: AsyncMutex::new(()),
            gtfs_gates: [AsyncMutex::new(()), AsyncMutex::new(())],
            last_error: Mutex::new(HashMap::new()),
        })
    }

    /// Credential mode, quotas, and cache flags. No secrets.
    ///
    /// # Returns
    ///
    /// Status JSON.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Config, Service};
    /// let dir = std::env::temp_dir().join(format!("njt-mcp-status-{}", std::process::id()));
    /// let config = Config::parse([
    ///     ("NJT_TOKEN", "replace-me"),
    ///     ("NJT_CACHE_DIR", dir.to_str().unwrap()),
    /// ])
    /// .unwrap();
    /// let service = Service::new(config).unwrap();
    /// let status = service.status();
    /// assert_eq!(status["credential_mode"], "token");
    /// assert!(!status.to_string().contains("replace-me"));
    /// let _ = std::fs::remove_dir_all(&dir);
    /// ```
    pub fn status(&self) -> Value {
        let snapshot = self.store.snapshot();
        let mut mints = serde_json::Map::new();
        let mut caps = serde_json::Map::new();
        let mut cached = serde_json::Map::new();
        let mut errors = serde_json::Map::new();
        let last = self
            .last_error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for (index, api) in Api::ALL.iter().enumerate() {
            mints.insert(api.as_str().to_string(), json!(snapshot.mints[index]));
            caps.insert(api.as_str().to_string(), json!(api.mint_cap()));
            cached.insert(
                api.as_str().to_string(),
                json!(snapshot.token_cached[index]),
            );
            errors.insert(
                api.as_str().to_string(),
                match last.get(api) {
                    Some(message) => Value::String(message.clone()),
                    None => Value::Null,
                },
            );
        }
        json!({
            "environment": self.config.environment().to_string(),
            "credential_mode": self.config.credential_mode().as_str(),
            "rail_origin": self.config.rail_origin(),
            "bus_origin": self.config.bus_origin(),
            "cache_dir": self.config.cache_dir(),
            "day": snapshot.day,
            "mints": mints,
            "mint_caps": caps,
            "schedule_fetches": snapshot.schedule_fetches,
            "schedule_cap": snapshot.schedule_cap,
            "live_calls": snapshot.live_calls,
            "live_cap": snapshot.live_cap,
            "rt_ttl_secs": self.config.rt_ttl_secs(),
            "gtfs_ttl_secs": self.config.gtfs_ttl_secs(),
            "tokens_cached": cached,
            "last_error": errors,
            "load_warning": self.store.load_warning(),
            "gtfs_cached": {
                "rail": self.store.root().join("gtfs-rail.zip").exists(),
                "bus": self.store.root().join("gtfs-bus.zip").exists(),
            },
            "token_note": "NJT_TOKEN is tried on every API. Tokens are minted per API, so one token may be rejected by products that did not issue it. Token-only mode cannot refresh an expired token.",
        })
    }

    /// Station list for rail.
    ///
    /// # Returns
    ///
    /// `{stations, count}`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when TrainData rejects the credentials or the quota is exhausted.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let stations = service.rail_stations().await?;
    /// let _ = stations;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rail_stations(&self) -> Result<Value, AppError> {
        let value = self
            .json_call(
                Api::TrainData,
                "/api/TrainData/getStationList",
                &[],
                CallClass::Reference,
            )
            .await?;
        Ok(shape::stations(&value))
    }

    /// Next departures for a station.
    ///
    /// A non-empty `line` calls `getTrainSchedule19Rec`. Otherwise the call is `getTrainSchedule`.
    ///
    /// # Arguments
    ///
    /// * `station` - station code from [`Service::rail_stations`].
    /// * `line` - optional line filter.
    /// * `stop_limit` - stops kept on each departure, clamped to 1..=40.
    ///
    /// # Returns
    ///
    /// Departure JSON.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] for a bad station code or an upstream failure.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let board = service.rail_departures("EX", None, 8).await?;
    /// let _ = board;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rail_departures(
        &self,
        station: &str,
        line: Option<&str>,
        stop_limit: usize,
    ) -> Result<Value, AppError> {
        let station = station_key(station)?;
        let stop_limit = stop_limit.clamp(1, 40);
        let line = line.map(str::trim).filter(|value| !value.is_empty());
        let line_owned = line.map(str::to_string);
        let mut fields = vec![("station", station.as_str())];
        if let Some(line) = line_owned.as_deref() {
            fields.push(("line", line));
        }
        let path = if line_owned.is_some() {
            "/api/TrainData/getTrainSchedule19Rec"
        } else {
            "/api/TrainData/getTrainSchedule"
        };
        let value = self
            .json_call(Api::TrainData, path, &fields, CallClass::Live)
            .await?;
        Ok(shape::departures(&value, line, stop_limit))
    }

    /// Station or line messages.
    ///
    /// # Arguments
    ///
    /// * `station` - optional station code.
    /// * `line` - optional line. At least one of the two is required.
    ///
    /// # Returns
    ///
    /// Message JSON.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when both inputs are empty or the upstream call fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let messages = service.rail_station_messages(Some("EX"), None).await?;
    /// let _ = messages;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rail_station_messages(
        &self,
        station: Option<&str>,
        line: Option<&str>,
    ) -> Result<Value, AppError> {
        let station = match station.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => station_key(value)?,
            None => String::new(),
        };
        let line = line.unwrap_or("").trim().to_string();
        if station.is_empty() && line.is_empty() {
            return Err(AppError::new("station or line is required"));
        }
        let mut fields = Vec::new();
        if !station.is_empty() {
            fields.push(("station", station.as_str()));
        }
        if !line.is_empty() {
            fields.push(("line", line.as_str()));
        }
        let value = self
            .json_call(
                Api::TrainData,
                "/api/TrainData/getStationMSG",
                &fields,
                CallClass::Live,
            )
            .await?;
        Ok(shape::station_messages(&value, &station, &line))
    }

    /// Stops for one train.
    ///
    /// # Arguments
    ///
    /// * `train` - train id from a departure.
    ///
    /// # Returns
    ///
    /// Stop JSON.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when `train` is empty or the upstream call fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let stops = service.rail_train_stops("9001").await?;
    /// let _ = stops;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rail_train_stops(&self, train: &str) -> Result<Value, AppError> {
        let train = required_text(train, "train", 32)?;
        let value = self
            .json_call(
                Api::TrainData,
                "/api/TrainData/getTrainStopList",
                &[("train", train.as_str())],
                CallClass::Live,
            )
            .await?;
        Ok(shape::train_stops(&value))
    }

    /// Trains that have moved in the recent window NJ Transit publishes.
    ///
    /// The upstream call returns the fleet. `train` and `line` filter the cached response.
    ///
    /// # Arguments
    ///
    /// * `train` - optional train id.
    /// * `line` - optional line substring.
    /// * `offset` - rows to skip.
    /// * `limit` - page size.
    ///
    /// # Returns
    ///
    /// Vehicle JSON.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] on upstream or quota failure.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let vehicles = service.rail_vehicles(None, None, 0, 20).await?;
    /// let _ = vehicles;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rail_vehicles(
        &self,
        train: Option<&str>,
        line: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Value, AppError> {
        let value = self
            .json_call(
                Api::TrainData,
                "/api/TrainData/getVehicleData",
                &[],
                CallClass::Live,
            )
            .await?;
        Ok(shape::rail_vehicles(&value, train, line, offset, limit))
    }

    /// 27-hour station schedule, fetched at most once per station per New York day.
    ///
    /// # Arguments
    ///
    /// * `station` - station code.
    /// * `njt_only` - maps to the boolean form field `NJTOnly`.
    /// * `offset` - rows to skip.
    /// * `limit` - page size.
    ///
    /// # Returns
    ///
    /// Schedule JSON. `cached` is true when today's disk file was reused.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the station code is invalid, the local cap of 4
    /// upstream calls is exhausted, or NJ Transit returns an error.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let schedule = service.rail_schedule("EX", false, 0, 20).await?;
    /// let _ = schedule;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rail_schedule(
        &self,
        station: &str,
        njt_only: bool,
        offset: usize,
        limit: usize,
    ) -> Result<Value, AppError> {
        let station = station_key(station)?;
        let _gate = self.schedule_gate.lock().await;
        let day = self.store.snapshot().day;
        if let Some(cached) = self.store.read_schedule(&station, njt_only)? {
            return Ok(shape::station_schedule(
                &cached, &station, njt_only, true, &day, offset, limit,
            ));
        }
        if let Some(message) = self.store.blocked_path("/api/TrainData/getStationSchedule") {
            return Err(self.fail(
                Api::TrainData,
                AppError::new(message).with_kind(ErrorKind::DailyLimit),
            ));
        }
        self.store.reserve_schedule()?;
        let flag = if njt_only { "true" } else { "false" };
        let value = self
            .json_call(
                Api::TrainData,
                "/api/TrainData/getStationSchedule",
                &[("station", station.as_str()), ("NJTOnly", flag)],
                CallClass::Schedule,
            )
            .await?;
        self.store.write_schedule(&station, njt_only, &value)?;
        Ok(shape::station_schedule(
            &value, &station, njt_only, false, &day, offset, limit,
        ))
    }

    /// Bus and light-rail locations.
    ///
    /// # Arguments
    ///
    /// * `mode` - `BUS`, `NLR`, `HBLR`, `RL`, or `ALL`. `None` means `ALL`.
    ///
    /// # Returns
    ///
    /// Location JSON.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the mode is unusable or BUSDV2 fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let locations = service.bus_locations(Some("ALL")).await?;
    /// let _ = locations;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bus_locations(&self, mode: Option<&str>) -> Result<Value, AppError> {
        let mode = mode_of(mode)?;
        let value = self
            .json_call(
                Api::BusDv2,
                "/api/BUSDV2/getLocations",
                &[("mode", mode.as_str())],
                CallClass::Reference,
            )
            .await?;
        Ok(shape::bus_locations(&value, &mode))
    }

    /// Routes for one mode.
    ///
    /// # Arguments
    ///
    /// * `mode` - same values as [`Service::bus_locations`].
    ///
    /// # Returns
    ///
    /// Route JSON.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] on a bad mode or upstream failure.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let routes = service.bus_routes(Some("BUS")).await?;
    /// let _ = routes;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bus_routes(&self, mode: Option<&str>) -> Result<Value, AppError> {
        let mode = mode_of(mode)?;
        let value = self
            .json_call(
                Api::BusDv2,
                "/api/BUSDV2/getBusRoutes",
                &[("mode", mode.as_str())],
                CallClass::Reference,
            )
            .await?;
        Ok(shape::bus_routes(&value, &mode))
    }

    /// Direction names for a route.
    ///
    /// # Arguments
    ///
    /// * `route` - route id from [`Service::bus_routes`].
    ///
    /// # Returns
    ///
    /// `{route, directions}`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when `route` is empty or the call fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let directions = service.bus_directions("99").await?;
    /// let _ = directions;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bus_directions(&self, route: &str) -> Result<Value, AppError> {
        let route = required_text(route, "route", 32)?;
        let value = self
            .json_call(
                Api::BusDv2,
                "/api/BUSDV2/getBusDirectionsData",
                &[("route", route.as_str())],
                CallClass::Reference,
            )
            .await?;
        Ok(shape::bus_directions(&value, &route))
    }

    /// Stops for a route, one stop name, or stops near a point.
    ///
    /// # Arguments
    ///
    /// * `query` - which BUSDV2 stop method to call.
    ///
    /// # Returns
    ///
    /// Stop JSON. `source` is `stops`, `stop_name`, or `nearby_stops`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the combination of fields is incomplete or the call fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let query = njt_mcp::service::BusStopQuery {
    ///     route: Some("99".into()),
    ///     direction: Some("To Example".into()),
    ///     name_contains: None,
    ///     stop: None,
    ///     latitude: None,
    ///     longitude: None,
    ///     radius: None,
    ///     mode: None,
    ///     limit: 20,
    ///     offset: 0,
    /// };
    /// let stops = service.bus_stops(&query).await?;
    /// let _ = stops;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bus_stops(&self, query: &BusStopQuery) -> Result<Value, AppError> {
        if query.latitude.is_some() || query.longitude.is_some() {
            let latitude = coordinate(query.latitude.as_deref().unwrap_or(""), "latitude")?;
            let longitude = coordinate(query.longitude.as_deref().unwrap_or(""), "longitude")?;
            let radius = radius_of(query.radius.unwrap_or(0))?;
            let route = required_text(query.route.as_deref().unwrap_or(""), "route", 32)?;
            let direction =
                required_text(query.direction.as_deref().unwrap_or(""), "direction", 64)?;
            let mode = mode_of(query.mode.as_deref())?;
            let radius_text = radius.to_string();
            let value = self
                .json_call(
                    Api::BusDv2,
                    "/api/BUSDV2/getBusLocationsData",
                    &[
                        ("route", route.as_str()),
                        ("direction", direction.as_str()),
                        ("lat", latitude.as_str()),
                        ("lon", longitude.as_str()),
                        ("radius", radius_text.as_str()),
                        ("mode", mode.as_str()),
                    ],
                    CallClass::Live,
                )
                .await?;
            return Ok(shape::bus_stops_near(&value));
        }
        if let Some(stop) = query
            .stop
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if query
                .route
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                let stop = required_text(stop, "stop", 32)?;
                let value = self
                    .json_call(
                        Api::BusDv2,
                        "/api/BUSDV2/getStopName",
                        &[("stopnum", stop.as_str())],
                        CallClass::Reference,
                    )
                    .await?;
                return Ok(shape::bus_stop_name(&value, &stop));
            }
        }
        let route = required_text(query.route.as_deref().unwrap_or(""), "route", 32)?;
        let direction = required_text(query.direction.as_deref().unwrap_or(""), "direction", 64)?;
        let mut fields = vec![("route", route.as_str()), ("direction", direction.as_str())];
        let name = query
            .name_contains
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(name) = name.as_deref() {
            fields.push(("namecontains", name));
        }
        let value = self
            .json_call(
                Api::BusDv2,
                "/api/BUSDV2/getStops",
                &fields,
                CallClass::Reference,
            )
            .await?;
        let mut shaped = shape::bus_stops(&value, &route, &direction);
        if let Some(rows) = shaped.get_mut("stops").and_then(Value::as_array_mut) {
            let paged = shape::page(std::mem::take(rows), query.offset, query.limit.max(1));
            shaped["stops"] = Value::Array(paged.rows);
            shaped["total"] = json!(paged.total);
            shaped["next_offset"] = json!(paged.next_offset);
        }
        Ok(shaped)
    }

    /// Departure board or route trips.
    ///
    /// # Arguments
    ///
    /// * `query` - board fields or a location/route pair.
    ///
    /// # Returns
    ///
    /// Departure JSON. `source` is `board` or `route_trips`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the required pair of fields is missing or the call fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let mut query = njt_mcp::service::BusDepartureQuery::default();
    /// query.stop = Some("10001".into());
    /// let board = service.bus_departures(&query).await?;
    /// let _ = board;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bus_departures(&self, query: &BusDepartureQuery) -> Result<Value, AppError> {
        if let Some(location) = query
            .location
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let location = required_text(location, "location", 32)?;
            let route = required_text(query.route.as_deref().unwrap_or(""), "route", 32)?;
            let value = self
                .json_call(
                    Api::BusDv2,
                    "/api/BUSDV2/getRouteTrips",
                    &[("location", location.as_str()), ("route", route.as_str())],
                    CallClass::Live,
                )
                .await?;
            return Ok(shape::bus_departures(&value, "route_trips"));
        }
        let stop = required_text(query.stop.as_deref().unwrap_or(""), "stop", 32)?;
        let mut owned = vec![("stop".to_string(), stop)];
        push_opt(&mut owned, "direction", query.direction.as_deref());
        push_opt(&mut owned, "route", query.route.as_deref());
        push_opt(&mut owned, "IP", query.ip.as_deref());
        if let Some(minutes) = query.minutes_requested {
            owned.push(("minutesRequested".to_string(), minutes.to_string()));
        }
        if let Some(variants) = query.max_variants {
            owned.push(("maxVariants".to_string(), variants.to_string()));
        }
        push_opt(&mut owned, "sortMode", query.sort_mode.as_deref());
        let refs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let value = self
            .json_call(Api::BusDv2, "/api/BUSDV2/getBusDV", &refs, CallClass::Live)
            .await?;
        Ok(shape::bus_departures(&value, "board"))
    }

    /// Stops on one bus trip.
    ///
    /// # Arguments
    ///
    /// * `timing_point_id` - from a departure row.
    /// * `sched_dep_time` - scheduled departure text, for example `6/22/2023 12:50:00 AM`.
    /// * `internal_trip_number` - from a departure row.
    ///
    /// # Returns
    ///
    /// Trip-stop JSON.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when a field is empty or the call fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let stops = service
    ///     .bus_trip_stops("TP1", "6/22/2023 12:50:00 AM", "INT1")
    ///     .await?;
    /// let _ = stops;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bus_trip_stops(
        &self,
        timing_point_id: &str,
        sched_dep_time: &str,
        internal_trip_number: &str,
    ) -> Result<Value, AppError> {
        let timing_point_id = required_text(timing_point_id, "timing_point_id", 64)?;
        let sched_dep_time = required_text(sched_dep_time, "sched_dep_time", 40)?;
        let internal_trip_number = required_text(internal_trip_number, "internal_trip_number", 64)?;
        let value = self
            .json_call(
                Api::BusDv2,
                "/api/BUSDV2/getTripStops",
                &[
                    ("timing_point_id", timing_point_id.as_str()),
                    ("sched_dep_time", sched_dep_time.as_str()),
                    ("internal_trip_number", internal_trip_number.as_str()),
                ],
                CallClass::Live,
            )
            .await?;
        Ok(shape::bus_trip_stops(
            &value,
            &timing_point_id,
            &sched_dep_time,
            &internal_trip_number,
        ))
    }

    /// Vehicles near a point.
    ///
    /// # Arguments
    ///
    /// * `latitude` - decimal latitude.
    /// * `longitude` - decimal longitude.
    /// * `radius` - integer radius. Swagger defaults this field to 1 and does not state a unit.
    /// * `mode` - `BUS`, `NLR`, `HBLR`, `RL`, or `ALL`.
    ///
    /// # Returns
    ///
    /// Vehicle JSON.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when a coordinate or radius is invalid or the call fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let vehicles = service.bus_vehicles_near("40.0", "-74.0", 1000, Some("BUS")).await?;
    /// let _ = vehicles;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bus_vehicles_near(
        &self,
        latitude: &str,
        longitude: &str,
        radius: u32,
        mode: Option<&str>,
    ) -> Result<Value, AppError> {
        let latitude = coordinate(latitude, "latitude")?;
        let longitude = coordinate(longitude, "longitude")?;
        let radius = radius_of(radius)?;
        let mode = mode_of(mode)?;
        let radius_text = radius.to_string();
        let value = self
            .json_call(
                Api::BusDv2,
                "/api/BUSDV2/getVehicleLocations",
                &[
                    ("lat", latitude.as_str()),
                    ("lon", longitude.as_str()),
                    ("radius", radius_text.as_str()),
                    ("mode", mode.as_str()),
                ],
                CallClass::Live,
            )
            .await?;
        Ok(shape::bus_vehicles_near(
            &value, &latitude, &longitude, radius, &mode,
        ))
    }

    /// GTFS-Realtime trip updates for rail or bus.
    ///
    /// # Arguments
    ///
    /// * `query` - service and filters.
    ///
    /// # Returns
    ///
    /// Decoded trip updates.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the account cannot read that GTFS API or the protobuf is invalid.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service, query: &njt_mcp::realtime::RtQuery) -> Result<(), njt_mcp::AppError> {
    /// let updates = service.gtfs_trip_updates(query).await?;
    /// let _ = updates;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn gtfs_trip_updates(&self, query: &RtQuery) -> Result<Value, AppError> {
        let feed = self.load_feed(query.transit, "trips").await?;
        Ok(realtime::project_trip_updates(&feed, query))
    }

    /// GTFS-Realtime vehicle positions.
    ///
    /// # Arguments
    ///
    /// * `query` - service and filters.
    ///
    /// # Returns
    ///
    /// Decoded positions.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the feed cannot be fetched or decoded.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service, query: &njt_mcp::realtime::RtQuery) -> Result<(), njt_mcp::AppError> {
    /// let vehicles = service.gtfs_vehicle_positions(query).await?;
    /// let _ = vehicles;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn gtfs_vehicle_positions(&self, query: &RtQuery) -> Result<Value, AppError> {
        let feed = self.load_feed(query.transit, "vehicles").await?;
        Ok(realtime::project_vehicles(&feed, query))
    }

    /// GTFS-Realtime alerts.
    ///
    /// # Arguments
    ///
    /// * `query` - service and filters.
    ///
    /// # Returns
    ///
    /// Decoded alerts.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the feed cannot be fetched or decoded.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service, query: &njt_mcp::realtime::RtQuery) -> Result<(), njt_mcp::AppError> {
    /// let alerts = service.gtfs_alerts(query).await?;
    /// let _ = alerts;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn gtfs_alerts(&self, query: &RtQuery) -> Result<Value, AppError> {
        let feed = self.load_feed(query.transit, "alerts").await?;
        Ok(realtime::project_alerts(&feed, query))
    }

    /// Search stops in the cached static GTFS zip.
    ///
    /// # Arguments
    ///
    /// * `transit` - rail or bus.
    /// * `query` - id, code, or name substring.
    /// * `limit` - page size.
    /// * `offset` - rows to skip.
    ///
    /// # Returns
    ///
    /// Stop JSON plus `service`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the zip cannot be downloaded or parsed.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let stops = service.gtfs_stops(njt_mcp::Transit::Rail, "Example", 20, 0).await?;
    /// let _ = stops;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn gtfs_stops(
        &self,
        transit: Transit,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Value, AppError> {
        let set = self.load_gtfs(transit).await?;
        let mut value = set.search_stops(query, limit, offset);
        value["service"] = json!(transit.as_str());
        Ok(value)
    }

    /// Search routes in the cached static GTFS zip.
    ///
    /// # Arguments
    ///
    /// * `transit` - rail or bus.
    /// * `query` - id or name substring.
    /// * `limit` - page size.
    /// * `offset` - rows to skip.
    ///
    /// # Returns
    ///
    /// Route JSON plus `service`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the zip cannot be downloaded or parsed.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let routes = service.gtfs_routes(njt_mcp::Transit::Bus, "99", 20, 0).await?;
    /// let _ = routes;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn gtfs_routes(
        &self,
        transit: Transit,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Value, AppError> {
        let set = self.load_gtfs(transit).await?;
        let mut value = set.search_routes(query, limit, offset);
        value["service"] = json!(transit.as_str());
        Ok(value)
    }

    /// One trip from the cached static GTFS zip.
    ///
    /// # Arguments
    ///
    /// * `transit` - rail or bus.
    /// * `trip` - `trip_id`, short name, or internal trip number.
    /// * `limit` - stop rows.
    /// * `offset` - stop rows to skip.
    ///
    /// # Returns
    ///
    /// Trip JSON plus `service`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when the zip is unavailable or the trip does not exist.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(service: &njt_mcp::Service) -> Result<(), njt_mcp::AppError> {
    /// let trip = service.gtfs_trip(njt_mcp::Transit::Rail, "T1", 20, 0).await?;
    /// let _ = trip;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn gtfs_trip(
        &self,
        transit: Transit,
        trip: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Value, AppError> {
        let trip = required_text(trip, "trip", 64)?;
        let set = self.load_gtfs(transit).await?;
        let mut value = set.trip(&trip, limit, offset)?;
        value["service"] = json!(transit.as_str());
        Ok(value)
    }

    /// POST a JSON method and parse the body.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// Used by the surrounding module. Doctests cover the public wrappers.
    async fn json_call(
        &self,
        api: Api,
        path: &str,
        fields: &[(&str, &str)],
        class: CallClass,
    ) -> Result<Value, AppError> {
        let bytes = self.fetch(api, path, fields, class).await?;
        serde_json::from_slice(&bytes).map_err(|_| {
            self.fail(
                api,
                AppError::new(format!(
                    "NJT {api} {path} did not return JSON. The account may lack access to this API."
                ))
                .with_kind(ErrorKind::AccessDenied),
            )
        })
    }

    /// POST with a token, retry once after an invalid token, and honor quotas.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// Used by the surrounding module. Doctests cover the public wrappers.
    async fn fetch(
        &self,
        api: Api,
        path: &str,
        fields: &[(&str, &str)],
        class: CallClass,
    ) -> Result<Vec<u8>, AppError> {
        if let Some(message) = self.store.blocked_path(path) {
            return Err(self.fail(api, AppError::new(message).with_kind(ErrorKind::DailyLimit)));
        }
        let key = cache_key(path, fields);
        let ttl = match class {
            CallClass::Reference => Duration::from_secs(REFERENCE_TTL_SECS),
            CallClass::Live => self.config.rt_ttl(),
            CallClass::Schedule | CallClass::Zip => Duration::from_secs(0),
        };
        if !ttl.is_zero() {
            if let Some(hit) = self.mem_get(&key, ttl) {
                return Ok(hit);
            }
        }
        if class == CallClass::Live {
            self.store
                .reserve_live()
                .map_err(|error| self.fail(api, error))?;
        }
        let mut rejected = Vec::new();
        for _ in 0..3 {
            let before = self.store.mint_count(api);
            let token = match self.tokens.acquire(api, &rejected).await {
                Ok(token) => token,
                Err(error) => return Err(self.fail(api, error)),
            };
            let minted = self.store.mint_count(api) > before;
            let mut owned = vec![("token".to_string(), token.clone())];
            for (name, value) in fields {
                owned.push(((*name).to_string(), (*value).to_string()));
            }
            let refs: Vec<(&str, &str)> = owned
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            let response = match self
                .client
                .post_form(&self.client.url(api, path), &refs, max_bytes_for(path))
                .await
            {
                Ok(response) => response,
                Err(error) => return Err(self.fail(api, error)),
            };
            match upstream::classify(response.status, &response.bytes) {
                Classified::Data(bytes) => {
                    if !ttl.is_zero() {
                        self.mem_put(key, bytes.clone());
                    }
                    return Ok(bytes);
                }
                Classified::InvalidToken => {
                    rejected.push(token);
                    if !self.tokens.can_mint() {
                        return Err(self.fail(api, AppError::token_expired(api)));
                    }
                    if minted {
                        return Err(self.fail(
                            api,
                            AppError::new(format!(
                                "NJT still rejected the token for {api} after minting a replacement."
                            )),
                        ));
                    }
                }
                Classified::DailyLimit { message } => {
                    let error = AppError::daily_limit(api, path, &message);
                    let error = self.scrub(error);
                    let _ = self.store.block_path(path, error.message());
                    return Err(self.fail(api, error));
                }
                other => return Err(self.fail(api, upstream::into_error(other, api, path))),
            }
        }
        Err(self.fail(
            api,
            AppError::new(format!("NJT rejected the token for {api}.")),
        ))
    }

    /// Download and decode one GTFS-Realtime method.
    ///
    /// # Arguments
    ///
    /// * `transit` - see the signature.
    /// * `kind` - see the signature.
    ///
    /// # Returns
    ///
    /// The value described above.
    ///
    /// # Errors
    ///
    /// Returns an error when the operation cannot finish. Credential values are not included.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// Used by the surrounding module. Doctests cover the public wrappers.
    async fn load_feed(
        &self,
        transit: Transit,
        kind: &str,
    ) -> Result<crate::gtfs_rt::FeedMessage, AppError> {
        let api = transit.gtfs_api();
        let path = feed_path(transit, kind);
        let bytes = self.fetch(api, path, &[], CallClass::Live).await?;
        realtime::decode_feed(&bytes).map_err(|error| self.fail(api, error))
    }

    /// Return the parsed GTFS zip, downloading it when the disk copy is stale.
    ///
    /// # Arguments
    ///
    /// * `transit` - see the signature.
    ///
    /// # Returns
    ///
    /// The value described above.
    ///
    /// # Errors
    ///
    /// Returns an error when the operation cannot finish. Credential values are not included.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// Used by the surrounding module. Doctests cover the public wrappers.
    async fn load_gtfs(&self, transit: Transit) -> Result<Arc<GtfsSet>, AppError> {
        let index = match transit {
            Transit::Rail => 0,
            Transit::Bus => 1,
        };
        let _gate = self.gtfs_gates[index].lock().await;
        if let Some(existing) = self.cached_gtfs(transit) {
            return Ok(existing);
        }
        let name = match transit {
            Transit::Rail => "gtfs-rail.zip",
            Transit::Bus => "gtfs-bus.zip",
        };
        if let Some(bytes) = self.store.read_bytes(name, self.config.gtfs_ttl_secs())? {
            let set = Arc::new(gtfs::parse_zip(&bytes)?);
            self.store_gtfs(transit, Arc::clone(&set));
            return Ok(set);
        }
        let api = transit.gtfs_api();
        let path = match transit {
            Transit::Rail => "/api/GTFSRT/getGTFS",
            Transit::Bus => "/api/GTFSG2/getGTFS",
        };
        let bytes = self.fetch(api, path, &[], CallClass::Zip).await?;
        let set = Arc::new(gtfs::parse_zip(&bytes).map_err(|error| self.fail(api, error))?);
        self.store.write_bytes(name, &bytes)?;
        self.store_gtfs(transit, Arc::clone(&set));
        Ok(set)
    }

    /// Return the in-memory GTFS set when this process already parsed it.
    ///
    /// # Arguments
    ///
    /// * `transit` - see the signature.
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
    fn cached_gtfs(&self, transit: Transit) -> Option<Arc<GtfsSet>> {
        let guard = self
            .gtfs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match transit {
            Transit::Rail => guard.rail.clone(),
            Transit::Bus => guard.bus.clone(),
        }
    }

    /// Keep a parsed GTFS set in memory.
    ///
    /// # Arguments
    ///
    /// * `transit` - see the signature.
    /// * `set` - see the signature.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// Used by the surrounding module. Doctests cover the public wrappers.
    fn store_gtfs(&self, transit: Transit, set: Arc<GtfsSet>) {
        let mut guard = self
            .gtfs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match transit {
            Transit::Rail => guard.rail = Some(set),
            Transit::Bus => guard.bus = Some(set),
        }
    }

    /// Return a fresh memory-cache body.
    ///
    /// # Arguments
    ///
    /// * `key` - see the signature.
    /// * `ttl` - see the signature.
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
    fn mem_get(&self, key: &str, ttl: Duration) -> Option<Vec<u8>> {
        let guard = self.mem.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = guard.get(key)?;
        let age = entry.at.elapsed();
        if age <= ttl {
            Some(entry.bytes.clone())
        } else {
            None
        }
    }

    /// Store a response body in the memory cache.
    ///
    /// # Arguments
    ///
    /// * `key` - see the signature.
    /// * `bytes` - see the signature.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// Used by the surrounding module. Doctests cover the public wrappers.
    fn mem_put(&self, key: String, bytes: Vec<u8>) {
        let mut guard = self.mem.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.insert(
            key,
            MemEntry {
                at: Instant::now(),
                bytes,
            },
        );
    }

    /// Remember the last error for an API and return the scrubbed error.
    ///
    /// # Arguments
    ///
    /// * `api` - see the signature.
    /// * `error` - see the signature.
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
    fn fail(&self, api: Api, error: AppError) -> AppError {
        let error = self.scrub(error);
        if let Ok(mut guard) = self.last_error.lock() {
            guard.insert(api, error.message().to_string());
        }
        error
    }

    /// Strip credential values out of an error before it is returned.
    ///
    /// # Arguments
    ///
    /// * `error` - see the signature.
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
    fn scrub(&self, error: AppError) -> AppError {
        let mut secrets = self.config.credentials().secrets();
        for api in Api::ALL {
            if let Some(token) = self.store.token(api) {
                secrets.push(token);
            }
        }
        let refs: Vec<&str> = secrets.iter().map(String::as_str).collect();
        redact_error(error, &refs)
    }
}

impl std::fmt::Debug for Service {
    /// Helper `fmt`.
    ///
    /// # Arguments
    ///
    /// * `formatter` - see the signature.
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
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Service")
            .field("credential_mode", &self.config.credential_mode())
            .field("environment", &self.config.environment())
            .finish()
    }
}

/// Build a memory-cache key from the path and form fields. The token is not included.
///
/// # Arguments
///
/// * `path` - see the signature.
/// * `fields` - see the signature.
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
fn cache_key(path: &str, fields: &[(&str, &str)]) -> String {
    let mut parts = vec![path.to_string()];
    for (name, value) in fields {
        parts.push(format!("{name}={value}"));
    }
    parts.join("\n")
}

/// Absolute path for one GTFS-Realtime method.
///
/// # Arguments
///
/// * `transit` - see the signature.
/// * `kind` - see the signature.
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
fn feed_path(transit: Transit, kind: &str) -> &'static str {
    match (transit, kind) {
        (Transit::Rail, "alerts") => "/api/GTFSRT/getAlerts",
        (Transit::Rail, "vehicles") => "/api/GTFSRT/getVehiclePositions",
        (Transit::Rail, _) => "/api/GTFSRT/getTripUpdates",
        (Transit::Bus, "alerts") => "/api/GTFSG2/getAlerts",
        (Transit::Bus, "vehicles") => "/api/GTFSG2/getVehiclePositions",
        (Transit::Bus, _) => "/api/GTFSG2/getTripUpdates",
    }
}

/// Trim a required string and reject values that are empty or too long.
///
/// # Arguments
///
/// * `value` - see the signature.
/// * `name` - see the signature.
/// * `max` - see the signature.
///
/// # Returns
///
/// The value described above.
///
/// # Errors
///
/// Returns an error when the operation cannot finish. Credential values are not included.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn required_text(value: &str, name: &str, max: usize) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max {
        return Err(AppError::new(format!(
            "{name} is empty or longer than {max} characters"
        )));
    }
    Ok(trimmed.to_string())
}

/// Normalize a bus mode to uppercase, defaulting to ALL.
///
/// # Arguments
///
/// * `value` - see the signature.
///
/// # Returns
///
/// The value described above.
///
/// # Errors
///
/// Returns an error when the operation cannot finish. Credential values are not included.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn mode_of(value: Option<&str>) -> Result<String, AppError> {
    let raw = value.unwrap_or("ALL").trim();
    let raw = if raw.is_empty() { "ALL" } else { raw };
    if raw.len() > 16 || !raw.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(AppError::new(
            "mode must be a short code such as BUS, NLR, HBLR, RL, or ALL",
        ));
    }
    Ok(raw.to_ascii_uppercase())
}

/// Accept a decimal latitude or longitude.
///
/// # Arguments
///
/// * `value` - see the signature.
/// * `name` - see the signature.
///
/// # Returns
///
/// The value described above.
///
/// # Errors
///
/// Returns an error when the operation cannot finish. Credential values are not included.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn coordinate(value: &str, name: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.parse::<f64>().is_err() {
        return Err(AppError::new(format!("{name} must be a decimal number")));
    }
    Ok(trimmed.to_string())
}

/// Accept a radius integer from 1 to 1000000.
///
/// # Arguments
///
/// * `value` - see the signature.
///
/// # Returns
///
/// The value described above.
///
/// # Errors
///
/// Returns an error when the operation cannot finish. Credential values are not included.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn radius_of(value: u32) -> Result<u32, AppError> {
    if value == 0 || value > 1_000_000 {
        Err(AppError::new("radius must be an integer from 1 to 1000000"))
    } else {
        Ok(value)
    }
}

/// Append a non-empty form field no longer than 64 characters.
///
/// # Arguments
///
/// * `fields` - see the signature.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn push_opt(fields: &mut Vec<(String, String)>, name: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        if value.chars().count() <= 64 {
            fields.push((name.to_string(), value.to_string()));
        }
    }
}
