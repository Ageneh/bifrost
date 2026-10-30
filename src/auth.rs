//! Admin authentication: password hashing, sessions, CSRF tokens, login
//! backoff and the setup code.
//!
//! Only `/bifrost` is protected by this; the Hue API stays open by design.
//! Nothing here logs a password, hash, token or setup code.

use std::collections::BTreeMap;
use std::io;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, TimeDelta, Utc};
use openssl::memcmp;
use openssl::rand::rand_bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::config::AuthConfig;
use crate::config_store::write_atomic;

/// Shortest admin password accepted, in characters.
pub const MIN_PASSWORD_CHARS: usize = 8;

/// Longest admin password accepted, in bytes. Hashing is deliberately slow,
/// so an unbounded input would be a cheap way to burn CPU.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// Random bytes in a session token.
const TOKEN_BYTES: usize = 32;

const SALT_BYTES: usize = 16;
const KEY_BYTES: usize = 32;
const SCRYPT_PREFIX: &str = "$scrypt$";

/// Bounds on the cost read back from a stored hash, so a hand-edited hash
/// cannot make every login take minutes or exhaust memory.
const MAX_LOG_N: u8 = 20;
const MAX_R: u32 = 32;
const MAX_P: u32 = 16;

/// Domain separation between the session id and the CSRF token, which are
/// both derived from the session token.
const CSRF_CONTEXT: &[u8] = b"bifrost-csrf\0";

/// Most sessions kept at once; the least recently used one goes first.
const MAX_SESSIONS: usize = 64;

/// A session's last use is written to disk at most this often, so a busy
/// UI does not rewrite the file on every request.
const TOUCH_PERSIST_INTERVAL: TimeDelta = TimeDelta::seconds(60);

/// Failed attempts from one source address before backoff starts.
const FREE_FAILURES: u32 = 3;
const BACKOFF_BASE: TimeDelta = TimeDelta::seconds(1);
const BACKOFF_MAX: TimeDelta = TimeDelta::minutes(15);

/// A source address with no failure for this long starts over.
const BACKOFF_FORGET_AFTER: TimeDelta = TimeDelta::hours(1);

/// Most source addresses tracked at once, so a flood of addresses cannot
/// grow the table without bound.
const MAX_TRACKED_SOURCES: usize = 4096;

/// Setup codes avoid characters that are easy to confuse (0/O, 1/I).
const SETUP_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const SETUP_CODE_GROUPS: usize = 3;
const SETUP_CODE_GROUP_LEN: usize = 4;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Wrong password")]
    WrongPassword,

    #[error("Wrong setup code")]
    WrongSetupCode,

    #[error("Too many failed attempts; try again in {} seconds", .0.as_secs().max(1))]
    RateLimited(Duration),

    #[error(
        "The password must be at least {MIN_PASSWORD_CHARS} characters and at most {MAX_PASSWORD_BYTES} bytes long"
    )]
    UnacceptablePassword,

    #[error("An admin password is already set")]
    AlreadySetUp,

    #[error("No admin password is set yet")]
    SetupRequired,

    #[error("Not logged in")]
    NotAuthenticated,

    #[error("Password hashing failed")]
    Hashing,

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yml::Error),
}

pub type AuthResult<T> = Result<T, AuthError>;

/// Source of the current time, injected so expiry can be tested.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0; N];
    // the OpenSSL CSPRNG only fails when it cannot be seeded, and then no
    // secret can be generated safely at all
    rand_bytes(&mut buf).expect("OpenSSL random number generator failed");
    buf
}

fn sha256_hex(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

/// Compare two secrets in constant time.
#[must_use]
pub fn secrets_match(a: &str, b: &str) -> bool {
    a.len() == b.len() && memcmp::eq(a.as_bytes(), b.as_bytes())
}

/// scrypt work factor: `N = 2^log_n`, block size `r`, parallelism `p`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashCost {
    pub log_n: u8,
    pub r: u32,
    pub p: u32,
}

impl HashCost {
    /// About 32 MiB and a few hundred milliseconds per hash.
    pub const DEFAULT: Self = Self {
        log_n: 15,
        r: 8,
        p: 1,
    };

    /// Memory scrypt needs for this cost, with headroom for OpenSSL.
    const fn max_memory(self) -> u64 {
        2 * 128 * (self.r as u64) * (1 << self.log_n) * (self.p as u64)
    }

    const fn is_bounded(self) -> bool {
        self.log_n > 0 && self.log_n <= MAX_LOG_N && self.r <= MAX_R && self.p <= MAX_P
    }
}

fn scrypt(password: &str, salt: &[u8], cost: HashCost) -> AuthResult<[u8; KEY_BYTES]> {
    let mut key = [0; KEY_BYTES];
    openssl::pkcs5::scrypt(
        password.as_bytes(),
        salt,
        1 << cost.log_n,
        u64::from(cost.r),
        u64::from(cost.p),
        cost.max_memory(),
        &mut key,
    )
    .map_err(|_| AuthError::Hashing)?;
    Ok(key)
}

/// Refuse passwords that are too short to resist guessing, or long enough
/// to make hashing expensive.
pub fn check_password_policy(password: &str) -> AuthResult<()> {
    if password.chars().count() < MIN_PASSWORD_CHARS || password.len() > MAX_PASSWORD_BYTES {
        return Err(AuthError::UnacceptablePassword);
    }
    Ok(())
}

/// Hash `password` with a fresh salt, as
/// `$scrypt$ln=<log_n>,r=<r>,p=<p>$<salt hex>$<key hex>`.
pub fn hash_password(password: &str, cost: HashCost) -> AuthResult<String> {
    let salt: [u8; SALT_BYTES] = random_bytes();
    let key = scrypt(password, &salt, cost)?;
    Ok(format!(
        "{SCRYPT_PREFIX}ln={},r={},p={}${}${}",
        cost.log_n,
        cost.r,
        cost.p,
        hex::encode(salt),
        hex::encode(key)
    ))
}

fn parse_hash(encoded: &str) -> Option<(HashCost, Vec<u8>, Vec<u8>)> {
    let rest = encoded.strip_prefix(SCRYPT_PREFIX)?;
    let mut parts = rest.split('$');
    let params = parts.next()?;
    let salt = hex::decode(parts.next()?).ok()?;
    let key = hex::decode(parts.next()?).ok()?;
    if parts.next().is_some() || salt.is_empty() || key.len() != KEY_BYTES {
        return None;
    }

    let mut cost = HashCost {
        log_n: 0,
        r: 0,
        p: 0,
    };
    for param in params.split(',') {
        match param.split_once('=')? {
            ("ln", value) => cost.log_n = value.parse().ok()?,
            ("r", value) => cost.r = value.parse().ok()?,
            ("p", value) => cost.p = value.parse().ok()?,
            _ => return None,
        }
    }

    cost.is_bounded().then_some((cost, salt, key))
}

/// Whether `password` matches a hash made by [`hash_password`]. A hash that
/// cannot be parsed matches nothing.
#[must_use]
pub fn verify_password(password: &str, encoded: &str) -> bool {
    if password.len() > MAX_PASSWORD_BYTES {
        return false;
    }
    let Some((cost, salt, expected)) = parse_hash(encoded) else {
        log::error!("The configured admin password hash cannot be read; no password matches it");
        return false;
    };
    scrypt(password, &salt, cost).is_ok_and(|key| memcmp::eq(&key, &expected))
}

/// Whether `encoded` is a password hash this version can verify.
#[must_use]
pub fn is_valid_hash(encoded: &str) -> bool {
    parse_hash(encoded).is_some()
}

/// How long sessions live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Expiry {
    pub idle: TimeDelta,
    pub absolute: TimeDelta,
}

impl Expiry {
    #[must_use]
    pub fn from_config(auth: &AuthConfig) -> Self {
        Self {
            idle: TimeDelta::minutes(i64::from(auth.idle_minutes.max(1))),
            absolute: TimeDelta::hours(i64::from(auth.session_hours.max(1))),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredSession {
    ip: IpAddr,
    created: DateTime<Utc>,
    last_seen: DateTime<Utc>,
}

impl StoredSession {
    fn is_live(&self, now: DateTime<Utc>, expiry: Expiry) -> bool {
        now < self.created + expiry.absolute && now < self.last_seen + expiry.idle
    }
}

/// The session file. Sessions are keyed by the SHA-256 of their token, so
/// the file alone cannot be replayed as a cookie.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionFile {
    sessions: BTreeMap<String, StoredSession>,
}

/// A freshly created session. `token` goes into the cookie, `csrf` to the
/// client; neither is stored.
pub struct NewSession {
    pub token: String,
    pub csrf: String,
}

/// Failed attempts from one source address.
#[derive(Clone, Copy, Debug)]
struct Attempts {
    failures: u32,
    last_failure: DateTime<Utc>,
    blocked_until: DateTime<Utc>,
}

/// Session store, login backoff and setup code, shared by every listener.
pub struct Auth {
    clock: Arc<dyn Clock>,
    cost: HashCost,
    sessions_path: Utf8PathBuf,
    sessions: Mutex<BTreeMap<String, StoredSession>>,
    attempts: Mutex<BTreeMap<IpAddr, Attempts>>,
    setup_code: Mutex<Option<String>>,
}

impl Auth {
    /// Open the session store at `sessions_path`. A file that cannot be
    /// read is reported and ignored: everyone logs in again.
    #[must_use]
    pub fn load(sessions_path: &Utf8Path, clock: Arc<dyn Clock>, cost: HashCost) -> Self {
        let sessions = match Self::read_sessions(sessions_path) {
            Ok(file) => file.sessions,
            Err(err) => {
                log::warn!("Ignoring unreadable session file {sessions_path}: {err}");
                BTreeMap::new()
            }
        };

        Self {
            clock,
            cost,
            sessions_path: sessions_path.to_path_buf(),
            sessions: Mutex::new(sessions),
            attempts: Mutex::new(BTreeMap::new()),
            setup_code: Mutex::new(None),
        }
    }

    fn read_sessions(path: &Utf8Path) -> AuthResult<SessionFile> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_yml::from_str(&text)?),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(SessionFile::default()),
            Err(err) => Err(err.into()),
        }
    }

    fn persist(&self, sessions: &BTreeMap<String, StoredSession>) -> AuthResult<()> {
        let file = SessionFile {
            sessions: sessions.clone(),
        };
        write_atomic(&self.sessions_path, serde_yml::to_string(&file)?.as_bytes())?;
        Ok(())
    }

    #[must_use]
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// Hash a new password at this store's cost. Slow by design; call it
    /// off the async executor.
    pub fn hash_password(&self, password: &str) -> AuthResult<String> {
        check_password_policy(password)?;
        hash_password(password, self.cost)
    }

    /// Start a session for a client at `ip`.
    pub fn create_session(&self, ip: IpAddr, expiry: Expiry) -> AuthResult<NewSession> {
        let token = hex::encode(random_bytes::<TOKEN_BYTES>());
        let now = self.now();

        let mut sessions = lock(&self.sessions);
        sessions.retain(|_, session| session.is_live(now, expiry));
        while sessions.len() >= MAX_SESSIONS {
            let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, session)| session.last_seen)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            sessions.remove(&oldest);
        }
        sessions.insert(
            session_id(&token),
            StoredSession {
                ip,
                created: now,
                last_seen: now,
            },
        );
        self.persist(&sessions)?;
        drop(sessions);

        Ok(NewSession {
            csrf: csrf_token(&token),
            token,
        })
    }

    /// Check a session token presented from `ip`. Returns the session's
    /// CSRF token when the session is live and bound to that address, and
    /// records the use.
    #[must_use]
    pub fn session(&self, token: &str, ip: IpAddr, expiry: Expiry) -> Option<String> {
        let id = session_id(token);
        let now = self.now();

        let mut sessions = lock(&self.sessions);
        let session = sessions.get_mut(&id)?;
        if !session.is_live(now, expiry) {
            sessions.remove(&id);
            let _ = self.persist(&sessions);
            return None;
        }
        // a stolen cookie replayed from another address is refused, but
        // does not end the owner's session
        if session.ip != ip {
            return None;
        }
        let persist = now - session.last_seen >= TOUCH_PERSIST_INTERVAL;
        session.last_seen = now;
        if persist {
            if let Err(err) = self.persist(&sessions) {
                log::warn!("Failed to save sessions: {err}");
            }
        }
        drop(sessions);

        Some(csrf_token(token))
    }

    /// End one session.
    // the lock is held across the write, so the file follows the map
    #[allow(clippy::significant_drop_tightening)]
    pub fn revoke(&self, token: &str) -> AuthResult<()> {
        let mut sessions = lock(&self.sessions);
        if sessions.remove(&session_id(token)).is_some() {
            self.persist(&sessions)?;
        }
        Ok(())
    }

    /// End every session.
    // the lock is held across the write, so the file follows the map
    #[allow(clippy::significant_drop_tightening)]
    pub fn revoke_all(&self) -> AuthResult<()> {
        let mut sessions = lock(&self.sessions);
        let had_sessions = !sessions.is_empty();
        sessions.clear();
        if had_sessions || self.sessions_path.exists() {
            self.persist(&sessions)?;
        }
        Ok(())
    }

    /// Number of live sessions.
    #[must_use]
    pub fn session_count(&self, expiry: Expiry) -> usize {
        let now = self.now();
        lock(&self.sessions)
            .values()
            .filter(|session| session.is_live(now, expiry))
            .count()
    }

    /// Refuse an attempt from `ip` while its backoff runs. Other addresses
    /// are never affected, so no host can lock the owner out.
    pub fn check_backoff(&self, ip: IpAddr) -> AuthResult<()> {
        let now = self.now();
        match lock(&self.attempts).get(&ip) {
            Some(attempts) if now < attempts.blocked_until => Err(AuthError::RateLimited(
                (attempts.blocked_until - now).to_std().unwrap_or_default(),
            )),
            _ => Ok(()),
        }
    }

    /// Record a failed attempt from `ip`; after [`FREE_FAILURES`], each one
    /// doubles the wait before the next attempt.
    #[allow(clippy::significant_drop_tightening)]
    pub fn record_failure(&self, ip: IpAddr) {
        let now = self.now();
        let mut attempts = lock(&self.attempts);

        if !attempts.contains_key(&ip) && attempts.len() >= MAX_TRACKED_SOURCES {
            attempts.retain(|_, entry| now - entry.last_failure < BACKOFF_FORGET_AFTER);
            if attempts.len() >= MAX_TRACKED_SOURCES {
                if let Some(oldest) = attempts
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_failure)
                    .map(|(ip, _)| *ip)
                {
                    attempts.remove(&oldest);
                }
            }
        }

        let entry = attempts.entry(ip).or_insert(Attempts {
            failures: 0,
            last_failure: now,
            blocked_until: now,
        });
        if now - entry.last_failure >= BACKOFF_FORGET_AFTER {
            entry.failures = 0;
        }
        entry.failures += 1;
        entry.last_failure = now;
        if entry.failures > FREE_FAILURES {
            let doublings = (entry.failures - FREE_FAILURES - 1).min(16);
            let delay = BACKOFF_BASE
                .checked_mul(1 << doublings)
                .unwrap_or(BACKOFF_MAX)
                .min(BACKOFF_MAX);
            entry.blocked_until = now + delay;
        }
    }

    /// A successful attempt clears the backoff for `ip`.
    pub fn record_success(&self, ip: IpAddr) {
        lock(&self.attempts).remove(&ip);
    }

    /// Create a new setup code, replacing any earlier one. The caller prints
    /// it to the console; it must never go to the log.
    #[must_use]
    pub fn issue_setup_code(&self) -> String {
        let mut code = String::new();
        for (index, byte) in random_bytes::<{ SETUP_CODE_GROUPS * SETUP_CODE_GROUP_LEN }>()
            .iter()
            .enumerate()
        {
            if index > 0 && index % SETUP_CODE_GROUP_LEN == 0 {
                code.push('-');
            }
            let index = usize::from(*byte) % SETUP_CODE_ALPHABET.len();
            code.push(char::from(SETUP_CODE_ALPHABET[index]));
        }
        *lock(&self.setup_code) = Some(code.clone());
        code
    }

    /// Whether `code` is the current setup code. Case, spaces and dashes do
    /// not matter.
    #[must_use]
    pub fn check_setup_code(&self, code: &str) -> bool {
        let Some(expected) = lock(&self.setup_code).clone() else {
            return false;
        };
        secrets_match(
            &normalize_setup_code(code),
            &normalize_setup_code(&expected),
        )
    }

    /// The setup code is single-use: drop it once a password is set.
    pub fn clear_setup_code(&self) {
        lock(&self.setup_code).take();
    }
}

fn normalize_setup_code(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// The id a session is stored under: a hash of its token.
fn session_id(token: &str) -> String {
    sha256_hex(&[token.as_bytes()])
}

/// The CSRF token of a session, derived from its token so it needs no
/// storage and dies with the session.
#[must_use]
pub fn csrf_token(session_token: &str) -> String {
    sha256_hex(&[CSRF_CONTEXT, session_token.as_bytes()])
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::{IpAddr, Ipv4Addr};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use chrono::TimeDelta;

    use crate::auth::{
        Auth, AuthError, Expiry, HashCost, MAX_PASSWORD_BYTES, check_password_policy, csrf_token,
        hash_password, is_valid_hash, verify_password,
    };
    use crate::testutil::{FAST_HASH, ManualClock, TempDir};

    const OWNER: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
    const OTHER: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 6));

    const EXPIRY: Expiry = Expiry {
        idle: TimeDelta::minutes(60),
        absolute: TimeDelta::hours(12),
    };

    fn auth(dir: &TempDir, clock: &Arc<ManualClock>) -> Auth {
        Auth::load(&dir.path().join("sessions.yaml"), clock.clone(), FAST_HASH)
    }

    #[test]
    fn password_round_trips() {
        let hash = hash_password("correct horse", FAST_HASH).unwrap();

        assert!(hash.starts_with("$scrypt$ln=4,r=8,p=1$"));
        assert!(!hash.contains("correct horse"));
        assert!(is_valid_hash(&hash));
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("correct horsf", &hash));
        assert!(!verify_password("", &hash));
    }

    #[test]
    fn same_password_hashes_differently() {
        assert_ne!(
            hash_password("correct horse", FAST_HASH).unwrap(),
            hash_password("correct horse", FAST_HASH).unwrap()
        );
    }

    #[test]
    fn unreadable_or_unbounded_hashes_match_nothing() {
        let hash = hash_password("correct horse", FAST_HASH).unwrap();
        let expensive = hash.replace("ln=4", "ln=40");

        for bad in ["", "plaintext", "$scrypt$ln=4,r=8,p=1$zz$zz", &expensive] {
            assert!(!is_valid_hash(bad), "{bad}");
            assert!(!verify_password("correct horse", bad), "{bad}");
        }
    }

    #[test]
    fn default_cost_is_the_documented_one() {
        assert_eq!(
            HashCost::DEFAULT,
            HashCost {
                log_n: 15,
                r: 8,
                p: 1
            }
        );
    }

    #[test]
    fn password_policy_bounds_length() {
        assert!(matches!(
            check_password_policy("short"),
            Err(AuthError::UnacceptablePassword)
        ));
        assert!(check_password_policy("eight ch").is_ok());
        assert!(check_password_policy(&"x".repeat(MAX_PASSWORD_BYTES + 1)).is_err());
    }

    #[test]
    fn session_is_bound_to_its_address() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let auth = auth(&dir, &clock);

        let session = auth.create_session(OWNER, EXPIRY).unwrap();

        assert_eq!(
            auth.session(&session.token, OWNER, EXPIRY),
            Some(session.csrf)
        );
        assert_eq!(auth.session(&session.token, OTHER, EXPIRY), None);
        // the replay from another address did not end the owner's session
        assert!(auth.session(&session.token, OWNER, EXPIRY).is_some());
        assert_eq!(auth.session("not-a-token", OWNER, EXPIRY), None);
    }

    #[test]
    fn idle_session_expires() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let auth = auth(&dir, &clock);
        let session = auth.create_session(OWNER, EXPIRY).unwrap();

        clock.advance(TimeDelta::minutes(59));
        assert!(auth.session(&session.token, OWNER, EXPIRY).is_some());
        clock.advance(TimeDelta::minutes(59));
        assert!(auth.session(&session.token, OWNER, EXPIRY).is_some());
        clock.advance(TimeDelta::minutes(61));
        assert!(auth.session(&session.token, OWNER, EXPIRY).is_none());
    }

    #[test]
    fn active_session_still_ends_at_absolute_expiry() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let auth = auth(&dir, &clock);
        let session = auth.create_session(OWNER, EXPIRY).unwrap();

        for _ in 0..23 {
            clock.advance(TimeDelta::minutes(30));
            assert!(auth.session(&session.token, OWNER, EXPIRY).is_some());
        }
        clock.advance(TimeDelta::minutes(29));
        assert!(auth.session(&session.token, OWNER, EXPIRY).is_some());
        clock.advance(TimeDelta::minutes(1));
        assert!(auth.session(&session.token, OWNER, EXPIRY).is_none());
    }

    #[test]
    fn sessions_survive_restart_without_storing_tokens() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let session = auth(&dir, &clock).create_session(OWNER, EXPIRY).unwrap();

        let path = dir.path().join("sessions.yaml");
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains(&session.token), "{text}");
        assert!(!text.contains(&session.csrf), "{text}");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let restarted = auth(&dir, &clock);
        assert_eq!(
            restarted.session(&session.token, OWNER, EXPIRY),
            Some(session.csrf)
        );
    }

    #[test]
    fn unreadable_session_file_is_ignored() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let _ = dir.write("sessions.yaml", "{ not: [valid");

        let auth = auth(&dir, &clock);

        assert_eq!(auth.session_count(EXPIRY), 0);
        assert!(auth.create_session(OWNER, EXPIRY).is_ok());
    }

    #[test]
    fn csrf_token_is_bound_to_the_session() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let auth = auth(&dir, &clock);

        let first = auth.create_session(OWNER, EXPIRY).unwrap();
        let second = auth.create_session(OWNER, EXPIRY).unwrap();

        assert_ne!(first.token, second.token);
        assert_ne!(first.csrf, second.csrf);
        assert_eq!(first.csrf, csrf_token(&first.token));
        assert_ne!(first.csrf, first.token);
    }

    #[test]
    fn revoke_ends_one_session_and_revoke_all_ends_every_one() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let auth = auth(&dir, &clock);
        let first = auth.create_session(OWNER, EXPIRY).unwrap();
        let second = auth.create_session(OTHER, EXPIRY).unwrap();
        let third = auth.create_session(OWNER, EXPIRY).unwrap();

        auth.revoke(&first.token).unwrap();
        assert!(auth.session(&first.token, OWNER, EXPIRY).is_none());
        assert!(auth.session(&second.token, OTHER, EXPIRY).is_some());

        auth.revoke_all().unwrap();
        assert!(auth.session(&second.token, OTHER, EXPIRY).is_none());
        assert!(auth.session(&third.token, OWNER, EXPIRY).is_none());
        assert_eq!(
            Auth::read_sessions(&dir.path().join("sessions.yaml"))
                .unwrap()
                .sessions
                .len(),
            0
        );
    }

    #[test]
    fn session_count_is_bounded() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let auth = auth(&dir, &clock);

        let first = auth.create_session(OWNER, EXPIRY).unwrap();
        for _ in 0..super::MAX_SESSIONS {
            clock.advance(TimeDelta::seconds(1));
            let _ = auth.create_session(OWNER, EXPIRY).unwrap();
        }

        assert_eq!(auth.session_count(EXPIRY), super::MAX_SESSIONS);
        assert!(auth.session(&first.token, OWNER, EXPIRY).is_none());
    }

    #[test]
    fn backoff_grows_per_source_and_never_locks_others_out() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let auth = auth(&dir, &clock);

        for _ in 0..super::FREE_FAILURES {
            auth.check_backoff(OTHER).unwrap();
            auth.record_failure(OTHER);
        }
        auth.check_backoff(OTHER).unwrap();
        auth.record_failure(OTHER);

        let Err(AuthError::RateLimited(first)) = auth.check_backoff(OTHER) else {
            panic!("expected backoff");
        };
        assert_eq!(first.as_secs(), 1);
        // the owner, on another address, is unaffected
        auth.check_backoff(OWNER).unwrap();

        clock.advance(TimeDelta::seconds(1));
        auth.check_backoff(OTHER).unwrap();
        auth.record_failure(OTHER);
        let Err(AuthError::RateLimited(second)) = auth.check_backoff(OTHER) else {
            panic!("expected backoff");
        };
        assert_eq!(second.as_secs(), 2);

        for _ in 0..40 {
            clock.advance(TimeDelta::hours(1) - TimeDelta::seconds(1));
            auth.record_failure(OTHER);
        }
        let Err(AuthError::RateLimited(capped)) = auth.check_backoff(OTHER) else {
            panic!("expected backoff");
        };
        assert_eq!(capped.as_secs(), 15 * 60);

        auth.record_success(OTHER);
        auth.check_backoff(OTHER).unwrap();
    }

    #[test]
    fn backoff_is_forgotten_after_a_quiet_hour() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let auth = auth(&dir, &clock);
        for _ in 0..=super::FREE_FAILURES {
            auth.record_failure(OTHER);
        }
        assert!(auth.check_backoff(OTHER).is_err());

        clock.advance(TimeDelta::hours(1));
        auth.record_failure(OTHER);

        auth.check_backoff(OTHER).unwrap();
    }

    #[test]
    fn setup_code_is_typable_and_forgiving() {
        let dir = TempDir::new();
        let clock = ManualClock::new();
        let auth = auth(&dir, &clock);

        assert!(!auth.check_setup_code(""));
        let code = auth.issue_setup_code();

        assert_eq!(code.len(), 14);
        assert!(auth.check_setup_code(&code));
        assert!(auth.check_setup_code(&code.to_lowercase().replace('-', " ")));
        assert!(!auth.check_setup_code("AAAA-AAAA-AAAA"));

        let next = auth.issue_setup_code();
        assert!(auth.check_setup_code(&next));
        assert!(next == code || !auth.check_setup_code(&code));

        auth.clear_setup_code();
        assert!(!auth.check_setup_code(&next));
    }
}
