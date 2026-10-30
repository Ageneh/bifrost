//! Entertainment streaming, as the admin API reports it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One running DTLS session, for one area.
///
/// `fps_in` and `fps_out` are measured over the last second of polling, not
/// per frame: the diagnostic screen asks once a second, and a per-frame
/// update would cost more than the stream itself.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StreamStats {
    /// The entertainment configuration this session streams.
    pub area: Uuid,
    pub started: DateTime<Utc>,
    /// Frames the sender delivered in the last second.
    pub fps_in: u32,
    /// Frames that reached zigbee2mqtt in the last second, after the
    /// area's throttle.
    pub fps_out: u32,
    /// Every frame this session has received.
    pub frames: u64,
    /// Frames the throttle dropped, so `frames - drops` is what was sent.
    pub drops: u64,
    /// Why the session ended, while it is still listed.
    pub last_error: Option<String>,
}
