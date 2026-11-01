//! Synthetic GTFS and GTFS-Realtime bytes for tests and doctests.
//!
//! Nothing in this module is an NJ Transit response. Names are placeholders
//! such as `Example Station`.

use std::io::{Cursor, Write};

use prost::Message;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::gtfs_rt::trip_update::{StopTimeEvent, StopTimeUpdate};
use crate::gtfs_rt::{
    alert, translated_string, FeedEntity, FeedHeader, FeedMessage, Position, TimeRange,
    TranslatedString, TripDescriptor, TripUpdate, VehicleDescriptor, VehiclePosition,
};

/// Zip containing one route, two stops, one trip, and `trips_internal.txt`.
///
/// # Returns
///
/// Zip bytes small enough to parse in a unit test.
///
/// # Errors
///
/// This function does not return an error. Zip writing uses only memory.
///
/// # Panics
///
/// Panics if the in-memory zip writer fails. That is a programming error in this fixture.
///
/// # Examples
///
/// ```
/// use njt_mcp::fixtures::example_zip;
/// let bytes = example_zip();
/// assert_eq!(&bytes[..2], b"PK");
/// ```
pub fn example_zip() -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(&mut cursor);
    let options = SimpleFileOptions::default();
    write_entry(&mut zip, options, "stops.txt", "stop_id,stop_name,stop_lat,stop_lon,stop_code\nEX1,Example Station,40.0,-74.0,100\nEX2,Example Terminal,40.1,-74.1,101\n");
    write_entry(
        &mut zip,
        options,
        "routes.txt",
        "route_id,route_short_name,route_long_name,route_type\nR1,99,Example Route,2\n",
    );
    write_entry(&mut zip, options, "trips.txt", "route_id,service_id,trip_id,trip_headsign,trip_short_name,direction_id\nR1,WK,T1,Example Terminal,9001,0\n");
    write_entry(&mut zip, options, "stop_times.txt", "trip_id,arrival_time,departure_time,stop_id,stop_sequence\nT1,10:00:00,10:00:00,EX1,1\nT1,10:20:00,10:20:00,EX2,2\n");
    write_entry(
        &mut zip,
        options,
        "trips_internal.txt",
        "trip_id,internal_trip_number\nT1,INT1\n",
    );
    zip.finish().expect("finish synthetic zip");
    cursor.into_inner()
}

/// Feed with one trip update, one vehicle, and one alert.
///
/// # Returns
///
/// A [`FeedMessage`] using only example ids.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::fixtures::example_feed;
/// assert_eq!(example_feed().header.gtfs_realtime_version, "2.0");
/// ```
#[allow(deprecated)]
pub fn example_feed() -> FeedMessage {
    FeedMessage {
        header: FeedHeader {
            gtfs_realtime_version: "2.0".to_string(),
            incrementality: Some(0),
            timestamp: Some(1_700_000_000),
            feed_version: None,
        },
        entity: vec![
            FeedEntity {
                id: "trip-1".to_string(),
                is_deleted: None,
                trip_update: Some(TripUpdate {
                    trip: TripDescriptor {
                        trip_id: Some("T1".to_string()),
                        route_id: Some("R1".to_string()),
                        direction_id: Some(0),
                        start_time: None,
                        start_date: Some("20260115".to_string()),
                        schedule_relationship: Some(0),
                        modified_trip: None,
                    },
                    vehicle: None,
                    stop_time_update: vec![StopTimeUpdate {
                        stop_sequence: Some(1),
                        stop_id: Some("EX1".to_string()),
                        arrival: Some(StopTimeEvent {
                            delay: Some(60),
                            time: Some(1_700_000_060),
                            uncertainty: Some(0),
                            scheduled_time: None,
                        }),
                        departure: None,
                        departure_occupancy_status: None,
                        schedule_relationship: Some(0),
                        stop_time_properties: None,
                    }],
                    timestamp: Some(1_700_000_000),
                    delay: Some(60),
                    trip_properties: None,
                }),
                vehicle: None,
                alert: None,
                shape: None,
                stop: None,
                trip_modifications: None,
            },
            FeedEntity {
                id: "vehicle-1".to_string(),
                is_deleted: None,
                trip_update: None,
                vehicle: Some(VehiclePosition {
                    trip: Some(TripDescriptor {
                        trip_id: Some("T1".to_string()),
                        route_id: Some("R1".to_string()),
                        direction_id: None,
                        start_time: None,
                        start_date: None,
                        schedule_relationship: None,
                        modified_trip: None,
                    }),
                    vehicle: Some(VehicleDescriptor {
                        id: Some("V1".to_string()),
                        label: Some("Example".to_string()),
                        license_plate: None,
                        wheelchair_accessible: None,
                    }),
                    position: Some(Position {
                        latitude: 40.0,
                        longitude: -74.0,
                        bearing: Some(90.0),
                        odometer: None,
                        speed: Some(10.0),
                    }),
                    current_stop_sequence: Some(1),
                    stop_id: Some("EX1".to_string()),
                    current_status: Some(2),
                    timestamp: Some(1_700_000_000),
                    congestion_level: None,
                    occupancy_status: None,
                    occupancy_percentage: None,
                    multi_carriage_details: Vec::new(),
                }),
                alert: None,
                shape: None,
                stop: None,
                trip_modifications: None,
            },
            FeedEntity {
                id: "alert-1".to_string(),
                is_deleted: None,
                trip_update: None,
                vehicle: None,
                alert: Some(crate::gtfs_rt::Alert {
                    active_period: vec![TimeRange {
                        start: Some(1_700_000_000),
                        end: Some(1_700_003_600),
                    }],
                    communication_period: Vec::new(),
                    impact_period: vec![TimeRange {
                        start: Some(1_700_000_000),
                        end: Some(1_700_003_600),
                    }],
                    informed_entity: vec![crate::gtfs_rt::EntitySelector {
                        agency_id: None,
                        route_id: Some("R1".to_string()),
                        route_type: None,
                        trip: None,
                        stop_id: Some("EX1".to_string()),
                        direction_id: None,
                    }],
                    cause: Some(alert::Cause::Maintenance as i32),
                    effect: Some(alert::Effect::ModifiedService as i32),
                    url: None,
                    header_text: Some(english("Example alert")),
                    description_text: Some(english("Example description")),
                    tts_header_text: None,
                    tts_description_text: None,
                    severity_level: Some(alert::SeverityLevel::Warning as i32),
                    image: None,
                    image_alternative_text: None,
                    cause_detail: None,
                    effect_detail: None,
                }),
                shape: None,
                stop: None,
                trip_modifications: None,
            },
        ],
    }
}

/// Protobuf bytes for [`example_feed`].
///
/// # Returns
///
/// Encoded bytes.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// ```
/// use njt_mcp::fixtures::example_feed_bytes;
/// assert!(!example_feed_bytes().is_empty());
/// ```
pub fn example_feed_bytes() -> Vec<u8> {
    example_feed().encode_to_vec()
}

/// Build an English GTFS-Realtime translation.
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
fn english(text: &str) -> TranslatedString {
    TranslatedString {
        translation: vec![translated_string::Translation {
            text: text.to_string(),
            language: Some("en".to_string()),
        }],
    }
}

/// Write one CSV member into the synthetic zip.
///
/// # Panics
///
/// Panics if the in-memory zip writer fails.
///
/// # Examples
///
/// Used by the surrounding module. Doctests cover the public wrappers.
fn write_entry<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
    name: &str,
    text: &str,
) {
    zip.start_file(name, options).expect("start synthetic file");
    zip.write_all(text.as_bytes())
        .expect("write synthetic file");
}
