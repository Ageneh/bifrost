//! What each entertainment stream is doing right now.
//!
//! The registry is the one place both sides of a stream meet: the DTLS
//! session counts what the sender delivered, the backend counts what reached
//! zigbee2mqtt and what its throttle dropped. A `std::sync::Mutex` is enough
//! — every operation is a few map lookups and no lock is held across an
//! await — and it keeps the per-frame path free of the async mutex that
//! guards the resources.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::sync::{Notify, broadcast};
use uuid::Uuid;

use bifrost_api::entertainment::StreamStats;

/// How often the frame rates are recomputed.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
struct Entry {
    stats: StreamStats,
    sampled_at: Instant,
    /// The counters at the last sample, so the next rate is a difference.
    sampled_in: u64,
    sampled_out: u64,
}

impl Entry {
    fn new(area: Uuid) -> Self {
        Self {
            stats: StreamStats {
                area,
                started: Utc::now(),
                fps_in: 0,
                fps_out: 0,
                frames: 0,
                drops: 0,
                last_error: None,
            },
            sampled_at: Instant::now(),
            sampled_in: 0,
            sampled_out: 0,
        }
    }

    /// Recompute the rates when a full interval has passed.
    ///
    /// Polling at 1 Hz, as the status endpoint is meant to be used, gives
    /// true one-second rates. Polling faster repeats the last measurement
    /// rather than dividing by a shorter interval, which would report a
    /// frame or two as hundreds of frames a second.
    fn resample(&mut self) {
        let elapsed = self.sampled_at.elapsed();
        if elapsed < SAMPLE_INTERVAL {
            return;
        }

        let seconds = elapsed.as_secs_f64();
        let forwarded = self.stats.frames - self.stats.drops;
        self.stats.fps_in = rate(self.stats.frames - self.sampled_in, seconds);
        self.stats.fps_out = rate(forwarded - self.sampled_out, seconds);
        self.sampled_in = self.stats.frames;
        self.sampled_out = forwarded;
        self.sampled_at = Instant::now();
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn rate(frames: u64, seconds: f64) -> u32 {
    (frames as f64 / seconds).round() as u32
}

/// How many stream changes a lagging admin socket may miss before it reads
/// the snapshot instead.
const EVENT_BUFFER: usize = 16;

/// Every stream that is running, by area.
#[derive(Debug)]
pub struct StreamRegistry {
    streams: Mutex<BTreeMap<Uuid, Entry>>,
    /// The areas a stop was asked for; the session ends on its next frame.
    stopping: Mutex<BTreeSet<Uuid>>,
    /// A change to the set of streams, for the admin socket. The payload is
    /// the snapshot itself, so a socket never has to read the map.
    events: broadcast::Sender<Vec<StreamStats>>,
    /// Wakes the sessions, so a stop takes effect on a sender that has gone
    /// quiet instead of waiting out its read timeout.
    wake: Arc<Notify>,
}

impl Default for StreamRegistry {
    fn default() -> Self {
        Self {
            streams: Mutex::new(BTreeMap::new()),
            stopping: Mutex::new(BTreeSet::new()),
            events: broadcast::channel(EVENT_BUFFER).0,
            wake: Arc::new(Notify::new()),
        }
    }
}

impl StreamRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Uuid, Entry>> {
        self.streams.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tell every admin socket what the streams are doing now.
    ///
    /// Only called when the set of streams changes, never per frame: a
    /// diagnostic screen does not need 120 updates a second.
    fn announce(&self) {
        let streams = self.snapshot();
        // a send error only means no admin socket is open
        let _ = self.events.send(streams);
    }

    /// The stream changes, for `/bifrost/ws`.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<StreamStats>> {
        self.events.subscribe()
    }

    /// Record that a session started for `area`, replacing anything the same
    /// area left behind.
    pub fn open(&self, area: Uuid) {
        self.lock().insert(area, Entry::new(area));
        self.stopping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&area);
        self.announce();
    }

    /// Drop the entry for `area`; a session that ended is no longer listed.
    pub fn close(&self, area: &Uuid) {
        self.lock().remove(area);
        self.stopping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(area);
        self.announce();
    }

    /// Ask the session streaming `area` to end, and answer whether there was
    /// one to ask.
    pub fn request_stop(&self, area: &Uuid) -> bool {
        if !self.lock().contains_key(area) {
            return false;
        }

        self.stopping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(*area);
        self.wake.notify_waiters();
        true
    }

    /// Resolve once a stop was asked for the stream of `area`.
    ///
    /// Cancel-safe: the session selects it against reading the next frame,
    /// so a sender that stopped sending still ends its stream when the
    /// screen asks it to.
    pub async fn wait_for_stop(&self, area: &Uuid) {
        loop {
            let notified = self.wake.notified();
            if self.stop_requested(area) {
                return;
            }
            notified.await;
        }
    }

    /// Whether a stop was asked for the stream of `area`.
    #[must_use]
    pub fn stop_requested(&self, area: &Uuid) -> bool {
        self.stopping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(area)
    }

    /// A frame arrived from the sender.
    pub fn frame_in(&self, area: &Uuid) {
        if let Some(entry) = self.lock().get_mut(area) {
            entry.stats.frames += 1;
        }
    }

    /// A frame was handed to zigbee2mqtt, or dropped by the throttle.
    pub fn frame_out(&self, area: &Uuid, forwarded: bool) {
        if forwarded {
            return;
        }

        if let Some(entry) = self.lock().get_mut(area) {
            entry.stats.drops += 1;
        }
    }

    /// Remember why a session ended.
    pub fn set_error(&self, area: &Uuid, error: &str) {
        if let Some(entry) = self.lock().get_mut(area) {
            entry.stats.last_error = Some(error.to_string());
        }
        self.announce();
    }

    /// The areas that are streaming right now.
    #[must_use]
    pub fn areas(&self) -> Vec<Uuid> {
        self.lock().keys().copied().collect()
    }

    /// How many sessions are running, which is what the limit counts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every stream, rates refreshed.
    #[must_use]
    pub fn snapshot(&self) -> Vec<StreamStats> {
        let mut streams = self.lock();
        streams
            .values_mut()
            .map(|entry| {
                entry.resample();
                entry.stats.clone()
            })
            .collect()
    }

    /// One stream, rates refreshed.
    #[must_use]
    pub fn get(&self, area: &Uuid) -> Option<StreamStats> {
        self.lock().get_mut(area).map(|entry| {
            entry.resample();
            entry.stats.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_is_listed_from_open_to_close() {
        let registry = StreamRegistry::new();
        let area = Uuid::new_v4();

        assert!(registry.is_empty());
        registry.open(area);
        assert_eq!(registry.areas(), [area]);
        assert_eq!(registry.get(&area).unwrap().frames, 0);

        registry.close(&area);
        assert!(registry.is_empty());
        assert!(registry.get(&area).is_none());
    }

    #[test]
    fn drops_are_counted_apart_from_frames() {
        let registry = StreamRegistry::new();
        let area = Uuid::new_v4();
        registry.open(area);

        registry.frame_in(&area);
        registry.frame_in(&area);
        registry.frame_in(&area);
        registry.frame_out(&area, true);
        registry.frame_out(&area, false);
        registry.frame_out(&area, false);

        let stats = registry.get(&area).unwrap();
        assert_eq!(stats.frames, 3);
        assert_eq!(stats.drops, 2);
    }

    #[test]
    fn a_frame_for_a_stream_that_ended_is_ignored() {
        let registry = StreamRegistry::new();
        let area = Uuid::new_v4();

        // no panic, no entry: a frame that arrives after the close is stale
        registry.frame_in(&area);
        registry.frame_out(&area, true);
        registry.set_error(&area, "gone");

        assert!(registry.is_empty());
    }

    #[test]
    fn a_stop_is_asked_for_once_and_cleared_with_the_stream() {
        let registry = StreamRegistry::new();
        let area = Uuid::new_v4();

        // nothing to stop
        assert!(!registry.request_stop(&area));
        assert!(!registry.stop_requested(&area));

        registry.open(area);
        assert!(registry.request_stop(&area));
        assert!(registry.stop_requested(&area));

        // the session ends and the request goes with it, so the next stream
        // on the same area is not stopped the moment it starts
        registry.close(&area);
        assert!(!registry.stop_requested(&area));

        registry.open(area);
        assert!(!registry.stop_requested(&area));
    }

    #[test]
    fn every_change_is_announced() {
        let registry = StreamRegistry::new();
        let mut events = registry.subscribe();
        let area = Uuid::new_v4();

        registry.open(area);
        let announced = events.try_recv().expect("the open was not announced");
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].area, area);

        registry.close(&area);
        let announced = events.try_recv().expect("the close was not announced");
        assert_eq!(announced.len(), 0, "the closed stream is still listed");
    }

    #[test]
    fn rates_are_measured_over_a_full_second() {
        let registry = StreamRegistry::new();
        let area = Uuid::new_v4();
        registry.open(area);

        for _ in 0..10 {
            registry.frame_in(&area);
        }

        // the sample interval has not passed: the rate stays at zero rather
        // than claiming ten frames a second
        assert_eq!(registry.get(&area).unwrap().fps_in, 0);
    }
}
