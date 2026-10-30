//! A bounded, in-memory copy of the log, for the log viewer.
//!
//! The buffer wraps whatever logger [`install`] is given: every record is
//! passed on unchanged and also kept in a ring of [`CAPACITY`] lines, which
//! the API serves and the websocket streams.
//!
//! Nothing may log from inside [`log::Log::log`]: the logger is called with
//! the logging machinery held, and a record logged from there recurses.
//!
//! The setup code is never logged (it goes to standard output), and a line
//! that still carries something that looks like a secret is redacted before
//! it is stored.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use chrono::Utc;
use tokio::sync::broadcast;

use bifrost_api::admin::LogRecord;

/// Lines the buffer keeps; older ones are dropped.
pub const CAPACITY: usize = 1000;

/// Subscribers a stream may fall behind by before it loses lines.
const BROADCAST_CAPACITY: usize = 256;

/// What replaces a secret-looking value in a stored line.
pub const REDACTED: &str = "<redacted>";

/// Keys whose value is a secret wherever they appear in a log line.
const SECRET_KEYS: &[&str] = &[
    "token",
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "access_token",
    "auth_token",
    "authorization",
];

static GLOBAL: OnceLock<Arc<LogBuffer>> = OnceLock::new();

/// The buffer the bridge is logging into.
///
/// Before [`install`] runs, or in a test that never installs a logger, this
/// is a detached buffer: it stores what is pushed into it and nothing else.
#[must_use]
pub fn shared() -> Arc<LogBuffer> {
    GLOBAL
        .get_or_init(|| Arc::new(LogBuffer::new(CAPACITY)))
        .clone()
}

/// Keep every record in a buffer, and pass it on to `inner`.
///
/// The buffer becomes [`shared`]. Returns an error when a logger is already
/// installed, which only a second call can cause.
pub fn install(inner: Box<dyn log::Log>) -> Result<Arc<LogBuffer>, log::SetLoggerError> {
    let buffer = Arc::new(LogBuffer::new(CAPACITY));
    let tee = Tee {
        buffer: buffer.clone(),
        inner,
    };

    log::set_boxed_logger(Box::new(tee))?;
    let _ = GLOBAL.set(buffer.clone());

    Ok(buffer)
}

/// The logger the bridge installs: the real logger, plus the buffer.
struct Tee {
    buffer: Arc<LogBuffer>,
    inner: Box<dyn log::Log>,
}

impl log::Log for Tee {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        self.buffer.push_record(record);
        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// A bounded ring of log lines, with a stream of every new one.
pub struct LogBuffer {
    records: Mutex<VecDeque<LogRecord>>,
    events: broadcast::Sender<LogRecord>,
    capacity: usize,
}

impl LogBuffer {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            records: Mutex::new(VecDeque::with_capacity(capacity)),
            events,
            capacity,
        }
    }

    /// Store `record` and hand it to every subscriber.
    pub fn push(&self, record: LogRecord) {
        {
            let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
            if records.len() == self.capacity {
                records.pop_front();
            }
            records.push_back(record.clone());
        }

        // a send error only means nobody is watching
        let _ = self.events.send(record);
    }

    /// Store a log record, with anything secret-looking redacted.
    pub fn push_record(&self, record: &log::Record<'_>) {
        self.push(LogRecord {
            timestamp: Utc::now().to_rfc3339(),
            level: record.level().to_string(),
            target: record.target().to_string(),
            message: scrub(&record.args().to_string()),
        });
    }

    /// The most recent `limit` lines, oldest first; every line without one.
    #[must_use]
    pub fn snapshot(&self, limit: Option<usize>) -> Vec<LogRecord> {
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let skip = limit.map_or(0, |limit| records.len().saturating_sub(limit));
        records.iter().skip(skip).cloned().collect()
    }

    /// Every line pushed from now on, until the receiver falls too far
    /// behind.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<LogRecord> {
        self.events.subscribe()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Replace the value of anything that reads as `key=value`, `key: value` or
/// `Bearer value` with [`REDACTED`].
///
/// A log line is not a configuration document, so this is deliberately
/// simple: it protects against the common shapes, and the rule remains that
/// a secret is never logged in the first place.
#[must_use]
pub fn scrub(line: &str) -> String {
    let mut out = Vec::with_capacity(line.split_whitespace().count());
    let mut redact_next = false;

    for word in line.split_whitespace() {
        match (redact_next, secret_marker(word)) {
            // the word after `token:` is the secret
            (true, None) => {
                out.push(REDACTED.to_string());
                redact_next = false;
            }
            // `Authorization: Bearer <token>`: keep the markers, take the value
            (_, Some(Marker::NextWord)) => {
                out.push(word.to_string());
                redact_next = true;
            }
            (true, Some(Marker::Inline { start, end })) => {
                out.push(redact(word, start, end));
                redact_next = false;
            }
            (false, Some(Marker::Inline { start, end })) => out.push(redact(word, start, end)),
            (false, None) => out.push(word.to_string()),
        }
    }

    out.join(" ")
}

/// Where the value inside `word` sits, so only it is replaced.
enum Marker {
    /// `token abc`: the value is the next word.
    NextWord,
    /// `token=abc`, or `?token=abc&x=1` inside a url.
    Inline { start: usize, end: usize },
}

/// What `word` says about a secret that follows or sits inside it.
fn secret_marker(word: &str) -> Option<Marker> {
    let lowered = word.to_ascii_lowercase();

    // a value in the same word, at the end of the word or before `&`
    for key in SECRET_KEYS.iter().chain(["bearer"].as_slice()) {
        for separator in ['=', ':'] {
            let needle = format!("{key}{separator}");
            let Some(at) = lowered.find(&needle) else {
                continue;
            };
            let start = at + needle.len();
            if start >= word.len() {
                return Some(Marker::NextWord);
            }
            let end = word[start..]
                .find('&')
                .map_or(word.len(), |offset| start + offset);
            return Some(Marker::Inline { start, end });
        }
    }

    // a marker on its own: `Bearer abc`, `token: abc`
    let bare = lowered.trim_end_matches([':', '=']);
    if bare == "bearer" || SECRET_KEYS.contains(&bare) {
        return Some(Marker::NextWord);
    }

    None
}

/// `word` with `start..end` replaced by [`REDACTED`].
fn redact(word: &str, start: usize, end: usize) -> String {
    format!("{}{REDACTED}{}", &word[..start], &word[end..])
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{CAPACITY, LogBuffer, REDACTED, Tee, scrub};
    use bifrost_api::admin::LogRecord;

    fn record(message: &str) -> LogRecord {
        LogRecord {
            timestamp: "2026-10-30T00:00:00Z".into(),
            level: "INFO".into(),
            target: "bifrost::test".into(),
            message: message.into(),
        }
    }

    #[test]
    fn the_ring_keeps_the_most_recent_lines() {
        let buffer = LogBuffer::new(3);

        for index in 0..5 {
            buffer.push(record(&format!("line {index}")));
        }

        let messages: Vec<String> = buffer
            .snapshot(None)
            .into_iter()
            .map(|record| record.message)
            .collect();
        assert_eq!(messages, ["line 2", "line 3", "line 4"]);
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn snapshot_takes_the_last_n_oldest_first() {
        let buffer = LogBuffer::new(CAPACITY);
        for index in 0..5 {
            buffer.push(record(&format!("line {index}")));
        }

        let messages: Vec<String> = buffer
            .snapshot(Some(2))
            .into_iter()
            .map(|record| record.message)
            .collect();
        assert_eq!(messages, ["line 3", "line 4"]);
    }

    #[tokio::test]
    async fn subscribers_see_every_new_line() {
        let buffer = LogBuffer::new(4);
        let mut stream = buffer.subscribe();

        buffer.push(record("hello"));

        assert_eq!(stream.recv().await.unwrap().message, "hello");
    }

    #[test]
    fn a_log_record_is_kept_with_its_level_and_target() {
        let buffer = LogBuffer::new(4);
        buffer.push_record(
            &log::Record::builder()
                .args(format_args!("connected to {}", "z2m"))
                .level(log::Level::Warn)
                .target("bifrost::backend")
                .build(),
        );

        let stored = &buffer.snapshot(None)[0];
        assert_eq!(stored.level, "WARN");
        assert_eq!(stored.target, "bifrost::backend");
        assert_eq!(stored.message, "connected to z2m");
    }

    /// A logger that counts what it is given, standing in for the console.
    struct Counting(Arc<AtomicUsize>);

    impl log::Log for Counting {
        fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, _record: &log::Record<'_>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn flush(&self) {}
    }

    #[test]
    fn the_wrapper_keeps_the_line_and_passes_it_on() {
        let buffer = Arc::new(LogBuffer::new(4));
        let counted = Arc::new(AtomicUsize::new(0));
        let tee = Tee {
            buffer: buffer.clone(),
            inner: Box::new(Counting(counted.clone())),
        };

        log::Log::log(
            &tee,
            &log::Record::builder()
                .args(format_args!("serving mac [00:11:22:33:44:55]"))
                .level(log::Level::Info)
                .target("bifrost::test")
                .build(),
        );

        assert_eq!(buffer.len(), 1);
        assert_eq!(counted.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn secrets_are_redacted_before_they_are_stored() {
        let buffer = LogBuffer::new(4);
        buffer.push_record(&log::Record::builder()
            .args(format_args!(
                "z2m url ws://host/api?token=s3cret&x=1 password: hunter2 Authorization: Bearer abc.def"
            ))
            .level(log::Level::Info)
            .target("bifrost::test")
            .build());

        let message = &buffer.snapshot(None)[0].message;
        assert!(!message.contains("s3cret"), "{message}");
        assert!(!message.contains("hunter2"), "{message}");
        assert!(!message.contains("abc.def"), "{message}");
        assert!(message.contains(REDACTED), "{message}");
    }

    #[test]
    fn ordinary_lines_are_unchanged() {
        for line in [
            "Serving mac [00:11:22:33:44:55]",
            "Applied configuration revision 4: 2 change(s), 0 restart(s) required",
            "no z2m servers configured; no lights will be available",
        ] {
            assert_eq!(scrub(line), line);
        }
    }

    #[test]
    fn scrubbing_covers_the_usual_shapes() {
        assert_eq!(scrub("token=abc"), format!("token={REDACTED}"));
        assert_eq!(scrub("token abc"), format!("token {REDACTED}"));
        assert_eq!(scrub("password: x"), format!("password: {REDACTED}"));
        assert_eq!(scrub("Bearer abc"), format!("Bearer {REDACTED}"));
        assert_eq!(scrub("password:x"), format!("password:{REDACTED}"));
        assert_eq!(
            scrub("Authorization: Bearer abc"),
            format!("Authorization: Bearer {REDACTED}")
        );
        assert_eq!(
            scrub("ws://h/api?token=abc&group=x"),
            format!("ws://h/api?token={REDACTED}&group=x")
        );
    }
}
