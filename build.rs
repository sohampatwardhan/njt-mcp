//! Compile the vendored GTFS-Realtime protobuf before the crate builds.
//!
//! # Panics
//!
//! Panics if `protoc` fails. The compiler is a build dependency, and a failed
//! codegen cannot produce a working binary.
//!
//! # Examples
//!
//! This file is invoked by Cargo. It is not called from the library.

fn main() {
    println!("cargo:rerun-if-changed=proto/gtfs-realtime.proto");
    prost_build::Config::new()
        .compile_protos(&["proto/gtfs-realtime.proto"], &["proto/"])
        .expect("compile gtfs-realtime.proto");
}
