//! NJ Transit products this server calls, and the rail/bus split used by GTFS tools.
//!
//! Tokens are minted per product. [`Api`] is the unit the token manager, quota
//! counters, and tool errors use. [`Transit`] is the smaller choice agents pass
//! to GTFS tools.

use crate::error::AppError;
use crate::limits::{BUS_MINT_CAP, RAIL_MINT_CAP};

/// One NJ Transit developer-portal API.
///
/// A token minted for one variant is not assumed valid for the others. Swagger
/// defines separate login methods, and this server stores a token per variant.
///
/// # Examples
///
/// ```
/// use njt_mcp::Api;
/// assert_eq!(Api::TrainData.login_path(), "/api/TrainData/getToken");
/// assert_eq!(Api::BusDv2.mint_cap(), 4);
/// assert_ne!(Api::TrainData.origin_host(), Api::BusDv2.origin_host());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Api {
    /// Rail Data at `/api/TrainData`.
    TrainData,
    /// Rail GTFS and GTFS-Realtime at `/api/GTFSRT`.
    GtfsRt,
    /// Bus GTFS G2 at `/api/GTFSG2`.
    GtfsG2,
    /// Bus and light-rail departure data at `/api/BUSDV2`.
    BusDv2,
}

/// Which family of hosts an [`Api`] uses.
///
/// # Examples
///
/// ```
/// use njt_mcp::{Api, OriginHost};
/// assert_eq!(Api::GtfsRt.origin_host(), OriginHost::Rail);
/// assert_eq!(Api::GtfsG2.origin_host(), OriginHost::Bus);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OriginHost {
    /// `raildata.njtransit.com` or `testraildata.njtransit.com`.
    Rail,
    /// `pcsdata.njtransit.com` or `testpcsdata.njtransit.com`.
    Bus,
}

/// Rail or bus GTFS, as chosen by an agent.
///
/// # Examples
///
/// ```
/// use njt_mcp::{Api, Transit};
/// assert_eq!(Transit::parse("rail").unwrap().gtfs_api(), Api::GtfsRt);
/// assert_eq!(Transit::parse("BUS").unwrap().as_str(), "bus");
/// assert!(Transit::parse("ferry").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transit {
    /// Rail static GTFS and GTFS-Realtime (`GTFSRT`).
    Rail,
    /// Bus static GTFS and GTFS-Realtime (`GTFSG2`), including light rail in that feed.
    Bus,
}

impl Api {
    /// Stable snake-case id used in cache files and status JSON.
    ///
    /// # Returns
    ///
    /// `train_data`, `gtfs_rt`, `gtfs_g2`, or `bus_dv2`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Api;
    /// assert_eq!(Api::GtfsRt.as_str(), "gtfs_rt");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TrainData => "train_data",
            Self::GtfsRt => "gtfs_rt",
            Self::GtfsG2 => "gtfs_g2",
            Self::BusDv2 => "bus_dv2",
        }
    }

    /// Short product name safe to show to an agent.
    ///
    /// # Returns
    ///
    /// The swagger product name, such as `TrainData`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Api;
    /// assert_eq!(Api::TrainData.label(), "TrainData");
    /// assert_eq!(Api::GtfsG2.label(), "GTFSG2");
    /// ```
    pub fn label(self) -> &'static str {
        match self {
            Self::TrainData => "TrainData",
            Self::GtfsRt => "GTFSRT",
            Self::GtfsG2 => "GTFSG2",
            Self::BusDv2 => "BUSDV2",
        }
    }

    /// Login path for this product.
    ///
    /// Rail products use `getToken`. Bus products use `authenticateUser`.
    /// Both are HTTP POST multipart forms with `username` and `password`.
    ///
    /// # Returns
    ///
    /// An absolute path beginning with `/api/`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Api;
    /// assert_eq!(Api::GtfsG2.login_path(), "/api/GTFSG2/authenticateUser");
    /// ```
    pub fn login_path(self) -> &'static str {
        match self {
            Self::TrainData => "/api/TrainData/getToken",
            Self::GtfsRt => "/api/GTFSRT/getToken",
            Self::GtfsG2 => "/api/GTFSG2/authenticateUser",
            Self::BusDv2 => "/api/BUSDV2/authenticateUser",
        }
    }

    /// Which configured origin this product is called on.
    ///
    /// # Returns
    ///
    /// [`OriginHost::Rail`] or [`OriginHost::Bus`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Api, OriginHost};
    /// assert_eq!(Api::TrainData.origin_host(), OriginHost::Rail);
    /// ```
    pub fn origin_host(self) -> OriginHost {
        match self {
            Self::TrainData | Self::GtfsRt => OriginHost::Rail,
            Self::GtfsG2 | Self::BusDv2 => OriginHost::Bus,
        }
    }

    /// Local mint cap for this product for one New York calendar day.
    ///
    /// # Returns
    ///
    /// [`RAIL_MINT_CAP`] for rail products and [`BUS_MINT_CAP`] for bus products.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Api;
    /// assert!(Api::TrainData.mint_cap() < 10);
    /// ```
    pub fn mint_cap(self) -> u32 {
        match self {
            Self::TrainData | Self::GtfsRt => RAIL_MINT_CAP,
            Self::GtfsG2 | Self::BusDv2 => BUS_MINT_CAP,
        }
    }

    /// Index into the per-API gate array.
    ///
    /// # Returns
    ///
    /// `0` through `3`.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Api;
    /// assert_eq!(Api::BusDv2.index(), 3);
    /// ```
    pub fn index(self) -> usize {
        match self {
            Self::TrainData => 0,
            Self::GtfsRt => 1,
            Self::GtfsG2 => 2,
            Self::BusDv2 => 3,
        }
    }

    /// Every product, in [`Api::index`] order.
    ///
    /// # Returns
    ///
    /// A four-element array.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Api;
    /// assert_eq!(Api::ALL.len(), 4);
    /// ```
    pub const ALL: [Self; 4] = [Self::TrainData, Self::GtfsRt, Self::GtfsG2, Self::BusDv2];
}

impl Transit {
    /// Parse `rail` or `bus`, ignoring ASCII case and surrounding space.
    ///
    /// # Arguments
    ///
    /// * `value` - agent input. Empty and unknown values are errors.
    ///
    /// # Returns
    ///
    /// The matching [`Transit`].
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when `value` is not `rail` or `bus`. The message
    /// does not echo values longer than 32 characters.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Transit;
    /// assert_eq!(Transit::parse(" Rail ").unwrap(), Transit::Rail);
    /// assert!(Transit::parse("").is_err());
    /// ```
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "rail" => Ok(Self::Rail),
            "bus" => Ok(Self::Bus),
            other => {
                let shown = if other.chars().count() > 32 {
                    "value".to_string()
                } else {
                    format!("{other:?}")
                };
                Err(AppError::new(format!(
                    "service must be \"rail\" or \"bus\", got {shown}"
                )))
            }
        }
    }

    /// Wire value agents already use: `rail` or `bus`.
    ///
    /// # Returns
    ///
    /// A static lowercase name.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::Transit;
    /// assert_eq!(Transit::Bus.as_str(), "bus");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rail => "rail",
            Self::Bus => "bus",
        }
    }

    /// API that serves this transit family's GTFS zip and GTFS-Realtime feeds.
    ///
    /// # Returns
    ///
    /// [`Api::GtfsRt`] or [`Api::GtfsG2`].
    ///
    /// # Panics
    ///
    /// This function does not panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use njt_mcp::{Api, Transit};
    /// assert_eq!(Transit::Bus.gtfs_api(), Api::GtfsG2);
    /// ```
    pub fn gtfs_api(self) -> Api {
        match self {
            Self::Rail => Api::GtfsRt,
            Self::Bus => Api::GtfsG2,
        }
    }
}

impl std::fmt::Display for Api {
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
        formatter.write_str(self.label())
    }
}

impl std::fmt::Display for Transit {
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
        formatter.write_str(self.as_str())
    }
}
