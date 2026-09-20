//! Tachyon Types.
//!
//! Shared low-level value types: fundamental identifiers and timestamps.
//! This crate must not gain networking, database, or runtime ownership
//! (see `docs/02_IMPLEMENTATION_SPEC.md` §1).

#![warn(unsafe_code)]

use std::fmt::{self, Display, Formatter};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Defines a newtype identifier wrapping a [`Uuid`] (v7).
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Generates a new time-ordered (v7) identifier.
            #[must_use]
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                Display::fmt(&self.0, f)
            }
        }
    };
}

define_id!(
    /// Identifies one executable unit of work (spec §3).
    TaskId
);
define_id!(
    /// Identifies one persistent user interaction context (spec §3).
    SessionId
);
define_id!(
    /// Identifies one node of a validated execution graph (spec §5).
    NodeId
);
define_id!(
    /// Identifies the workspace a task operates in.
    WorkspaceId
);
define_id!(
    /// Identifies one durable or wire event (spec §16).
    EventId
);
define_id!(
    /// Identifies one policy approval request (spec §33).
    ApprovalId
);
define_id!(
    /// Identifies one recoverable multi-file mutation batch (spec §20).
    MutationBatchId
);

/// Identifies a model or judgment provider implementation (spec §25).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderId(pub String);

/// Identifies a native capability in the registry (spec §28).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityId(pub String);

/// Content-addressed artifact identity: BLAKE3 hash in hexadecimal.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactId(pub String);

impl Display for ProviderId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Display for CapabilityId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Display for ArtifactId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Microseconds since the Unix epoch, UTC.
///
/// Stored as an integer for stable ordering and serialization; displayed as
/// RFC 3339 (`1970-01-01T00:00:00Z` style) for human and log consumption.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// Current wall-clock time. Panics only if the system clock predates 1970.
    #[must_use]
    pub fn now() -> Self {
        let micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_micros();
        Self(i64::try_from(micros).unwrap_or(i64::MAX))
    }

    /// Wraps a raw microsecond count.
    #[must_use]
    pub fn from_micros(micros: i64) -> Self {
        Self(micros)
    }

    /// Returns the raw microsecond count.
    #[must_use]
    pub fn as_micros(self) -> i64 {
        self.0
    }
}

impl Display for Timestamp {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&format_rfc3339(self.0))
    }
}

/// Formats `micros` as RFC 3339 UTC, trimming trailing fractional zeros.
fn format_rfc3339(micros: i64) -> String {
    const MICROS_PER_SEC: i64 = 1_000_000;
    const SECS_PER_DAY: i64 = 86_400;
    let secs = micros.div_euclid(MICROS_PER_SEC);
    let frac = micros.rem_euclid(MICROS_PER_SEC);
    let days = secs.div_euclid(SECS_PER_DAY);
    let secs_of_day = secs.rem_euclid(SECS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    let mut out = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}");
    if frac != 0 {
        let mut digits = format!("{frac:06}");
        while digits.ends_with('0') {
            digits.pop();
        }
        out.push('.');
        out.push_str(&digits);
    }
    out.push('Z');
    out
}

/// Converts days since the Unix epoch to (year, month, day).
/// Proleptic Gregorian calendar (Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    // By construction `m` is 1..=12 and `d` is 1..=31; the conversions below
    // cannot fail and exist only to satisfy checked-cast lints.
    let month = u32::try_from(m).expect("month out of range 1..=12");
    let day = u32::try_from(d).expect("day out of range 1..=31");
    (if m <= 2 { y + 1 } else { y }, month, day)
}

#[cfg(test)]
mod tests {
    use super::{EventId, TaskId, Timestamp};
    use serde_json::{from_str, to_string};
    use uuid::Uuid;

    #[test]
    fn generated_ids_are_unique_and_round_trip() {
        let a = TaskId::generate();
        let b = TaskId::generate();
        assert_ne!(a, b);
        let json = to_string(&a).unwrap();
        let back: TaskId = from_str(&json).unwrap();
        assert_eq!(a, back);
        assert_eq!(a.to_string(), a.0.to_string());
        assert_eq!(Uuid::parse_str(&a.to_string()).unwrap(), a.0);
    }

    #[test]
    fn event_id_displays_as_uuid() {
        let id = EventId::generate();
        assert_eq!(id.to_string().len(), 36);
    }

    #[test]
    fn timestamp_formats_known_values() {
        assert_eq!(
            Timestamp::from_micros(0).to_string(),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            Timestamp::from_micros(1_500_000).to_string(),
            "1970-01-01T00:00:01.5Z"
        );
        assert_eq!(
            Timestamp::from_micros(-1).to_string(),
            "1969-12-31T23:59:59.999999Z"
        );
        assert_eq!(
            Timestamp::from_micros(1_000_000_000_123_456).to_string(),
            "2001-09-09T01:46:40.123456Z"
        );
    }

    #[test]
    fn timestamp_orders_and_round_trips() {
        let earlier = Timestamp::from_micros(100);
        let later = Timestamp::now();
        assert!(earlier < later);
        assert_eq!(earlier.as_micros(), 100);
        let json = to_string(&later).unwrap();
        let back: Timestamp = from_str(&json).unwrap();
        assert_eq!(later, back);
    }
}
