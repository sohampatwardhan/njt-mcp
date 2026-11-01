//! Local rate-limit caps and the New York service day used to reset them.
//!
//! The caps sit under the published NJ Transit budgets so a bug or a chatty
//! agent fails here instead of exhausting the developer's account. They are
//! not configurable.

use chrono::{DateTime, TimeZone, Utc};

/// Login calls this process will make for one rail API on one service day.
///
/// NJ Transit counts `getToken` and `isValidToken` together, about 10 per day.
/// This server never calls `isValidToken`.
pub const RAIL_MINT_CAP: u32 = 8;

/// Login calls this process will make for one bus API on one service day.
///
/// Bus tokens last about 24 hours, so a small cap is enough.
pub const BUS_MINT_CAP: u32 = 4;

/// Upstream `getStationSchedule` calls per service day.
///
/// NJ Transit allows 5 per day. The disk cache serves repeats without a call.
pub const SCHEDULE_FETCH_CAP: u32 = 4;

/// Upstream live calls per service day across rail and bus current-data methods.
///
/// Vehicle and departure data is published with a 40,000 per day budget. NJ
/// Transit asks that it be requested on demand rather than polled.
pub const LIVE_CALL_CAP: u32 = 2_000;

/// How long reference lists stay in memory, in seconds (6 hours).
pub const REFERENCE_TTL_SECS: u64 = 6 * 60 * 60;

/// Default live-response cache TTL, in seconds.
pub const DEFAULT_RT_TTL_SECS: u64 = 30;

/// Default static GTFS zip TTL, in seconds (12 hours).
pub const DEFAULT_GTFS_TTL_SECS: u64 = 12 * 60 * 60;

/// Inclusive minimum for `NJT_RT_TTL`.
pub const MIN_RT_TTL_SECS: u64 = 5;

/// Inclusive maximum for `NJT_RT_TTL`.
pub const MAX_RT_TTL_SECS: u64 = 300;

/// Inclusive maximum for `NJT_GTFS_TTL` (7 days). `0` means keep the first zip.
pub const MAX_GTFS_TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// Maximum accepted JSON body, in bytes.
pub const JSON_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Maximum accepted GTFS zip, in bytes.
pub const ZIP_MAX_BYTES: usize = 80 * 1024 * 1024;

/// Maximum accepted GTFS-Realtime body, in bytes.
pub const PROTO_MAX_BYTES: usize = 20 * 1024 * 1024;

/// Default HTTP listen address.
pub const DEFAULT_BIND: &str = "127.0.0.1:8092";

/// Production rail origin, with no path.
pub const PROD_RAIL_ORIGIN: &str = "https://raildata.njtransit.com";

/// Production bus origin, with no path.
pub const PROD_BUS_ORIGIN: &str = "https://pcsdata.njtransit.com";

/// Test rail origin, with no path.
pub const TEST_RAIL_ORIGIN: &str = "https://testraildata.njtransit.com";

/// Test bus origin, with no path.
pub const TEST_BUS_ORIGIN: &str = "https://testpcsdata.njtransit.com";

/// Calendar day in America/New_York for a Unix timestamp.
///
/// NJ Transit daily quotas are treated as this timezone. DST is honored.
///
/// # Arguments
///
/// * `unix` - seconds since the Unix epoch. Values that `chrono` cannot
///   represent return `1970-01-01`.
///
/// # Returns
///
/// `YYYY-MM-DD` in America/New_York.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::service_day;
/// let afternoon = chrono::DateTime::parse_from_rfc3339("2026-01-15T15:00:00Z")
///     .unwrap()
///     .timestamp();
/// assert_eq!(service_day(afternoon), "2026-01-15");
/// let still_yesterday = chrono::DateTime::parse_from_rfc3339("2026-01-15T03:30:00Z")
///     .unwrap()
///     .timestamp();
/// assert_eq!(service_day(still_yesterday), "2026-01-14");
/// ```
pub fn service_day(unix: i64) -> String {
    let Some(utc) = DateTime::<Utc>::from_timestamp(unix, 0) else {
        return "1970-01-01".to_string();
    };
    utc.with_timezone(&chrono_tz::America::New_York)
        .format("%Y-%m-%d")
        .to_string()
}

/// Current Unix time, or `0` when the system clock is before the epoch.
///
/// # Returns
///
/// Whole seconds since the Unix epoch.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::now_unix;
/// assert!(now_unix() > 1_700_000_000);
/// ```
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Current New York service day.
///
/// # Returns
///
/// The same string shape as [`service_day`].
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::limits::today_service_day;
/// let day = today_service_day();
/// assert_eq!(day.len(), 10);
/// assert_eq!(&day[4..5], "-");
/// ```
pub fn today_service_day() -> String {
    service_day(now_unix())
}

/// Convert a timezone-aware civil time, used by unit tests that want a fixed zone.
///
/// This is a tiny wrapper so callers do not need to import [`TimeZone`] themselves
/// when reading the module. Production code uses [`service_day`].
///
/// # Arguments
///
/// * `year` - full year.
/// * `month` - month from 1 to 12.
/// * `day` - day of the month.
/// * `hour` - hour from 0 to 23 in America/New_York.
///
/// # Returns
///
/// Unix seconds for that civil time, or `None` when the date does not exist.
///
/// # Panics
///
/// This function does not panic. Invalid dates return `None`, including the
/// missing DST hour.
///
/// # Examples
///
/// ```
/// use njt_mcp::limits::new_york_unix;
/// let unix = new_york_unix(2026, 1, 15, 10).unwrap();
/// assert_eq!(njt_mcp::service_day(unix), "2026-01-15");
/// ```
pub fn new_york_unix(year: i32, month: u32, day: u32, hour: u32) -> Option<i64> {
    chrono_tz::America::New_York
        .with_ymd_and_hms(year, month, day, hour, 0, 0)
        .single()
        .map(|instant| instant.timestamp())
}
