//! One in-memory cache of yt-dlp's `--dump-single-json` document, keyed by URL.
//!
//! Why this exists: on a real network `yt-dlp --dump-single-json` was measured
//! at 33–47 seconds per call for a single YouTube URL, and the app made that
//! call twice for the same URL — once when the trim panel opened
//! (`probe::resolve_preview`) and once when the job was enqueued
//! (`probe::fetch_job_metadata`) — plus a full call again on every re-open.
//! Nothing about the yt-dlp invocation itself is tuned here; the fix is to stop
//! making the same call more than once.
//!
//! Why a module of its own rather than a static inside `probe`: the cache knows
//! nothing about yt-dlp. It is handed a future that produces a
//! `serde_json::Value` and it decides whether that future needs to run at all,
//! which is exactly what makes it unit-testable without the network — every
//! test below injects a counting fake in place of the subprocess. It is also
//! shared *state*, and state in this app is an `Arc<Mutex<_>>` created in
//! `run()` and handed to Tauri's `.manage()`, the way `jobs::SharedJobs` is —
//! not a global.
//!
//! The two invariants that matter:
//!
//! * **The probe never runs under the lock.** The lock is taken to look in the
//!   maps and to register or clear an in-flight marker; those critical sections
//!   contain no `.await` and no I/O. A 35-second subprocess held under this
//!   mutex would freeze every other caller of it.
//! * **A second caller for a URL already being probed waits for the first**
//!   rather than starting its own. This is not hypothetical: `enqueue_job`
//!   spawns the metadata fetch while the preview probe for the same URL may
//!   still be running, and two 35-second probes racing is the worst outcome
//!   available.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// How old a cached document may be and still be used to *play* something.
///
/// The formats in the document carry signed, time-limited URLs — YouTube's
/// `expire` query parameter is a unix timestamp typically a few hours out. A
/// stale entry would therefore hand the preview a dead stream URL, which fails
/// in the webview as a black rectangle rather than as anything diagnosable, so
/// the window here is deliberately a small fraction of the shortest lifetime
/// observed: ten minutes against hours. That still collapses the common case
/// this cache exists for — open the trim panel, set the cut points, enqueue —
/// into a single probe, because that sequence takes seconds, not minutes.
pub const PLAYBACK_MAX_AGE: Duration = Duration::from_secs(10 * 60);

/// How old a cached document may be and still be used for *title and thumbnail*.
///
/// Those two fields do not expire the way a format URL does: a video's title
/// and poster frame are the same an hour later, and the worst case for a stale
/// one is a queued job labelled with a title the uploader has since changed.
/// So the metadata path accepts a far older entry than the playback path — the
/// distinction costs one parameter at the call site and nothing else, and it
/// means enqueueing a URL previewed half an hour ago still costs no probe.
///
/// Doubles as the prune horizon: past this, no caller would accept the entry,
/// so it is not worth the memory.
pub const METADATA_MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);

/// Hard cap on retained documents.
///
/// A `--dump-single-json` document for a YouTube video is on the order of a
/// megabyte (the `formats` array dominates), so this is a memory bound first
/// and a hit-rate tradeoff second. Sixteen is far more than the handful of URLs
/// one session actually revisits.
const CAPACITY: usize = 16;

/// The parsed document, shared rather than cloned: callers only read it, and
/// the whole point is to avoid paying for it twice.
pub type Document = Arc<serde_json::Value>;

struct Entry {
    doc: Document,
    /// When the probe that produced this returned. Age is measured from here,
    /// not from first use, because it is the *format URLs* that are ageing.
    stored: Instant,
    /// Insertion order, for eviction. A monotone counter rather than `stored`
    /// alone so that two entries stored in the same instant still have a
    /// defined victim.
    seq: u64,
}

#[derive(Default)]
pub struct MetadataCache {
    entries: HashMap<String, Entry>,
    /// URLs with a probe running right now, and the channel its result will be
    /// published on. Present only for the duration of that probe: the leader's
    /// `InFlightGuard` removes the URL on every exit path, panic included.
    in_flight: HashMap<String, broadcast::Sender<Result<Document, String>>>,
    next_seq: u64,
}

/// Managed by Tauri and cloned into spawned tasks, exactly like
/// `jobs::SharedJobs`.
pub type SharedMetadataCache = Arc<Mutex<MetadataCache>>;

pub fn new_shared() -> SharedMetadataCache {
    Arc::new(Mutex::new(MetadataCache::default()))
}

/// Poison recovery instead of `.unwrap()`.
///
/// Two reasons this one mutex differs from the rest of the app's. The data
/// behind it is two maps and a counter, and no critical section here can leave
/// them half-written — none of them calls anything that can panic. And
/// `InFlightGuard::drop` takes this lock *while unwinding* from a panicking
/// probe; a second panic there would abort the process, and worse, would leave
/// every waiter for that URL blocked forever.
fn lock(cache: &SharedMetadataCache) -> MutexGuard<'_, MetadataCache> {
    cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl MetadataCache {
    /// Drops what nobody would accept any more, then evicts down to `CAPACITY`
    /// oldest-first.
    ///
    /// Insertion order, not least-recently-used: an entry's value decays with
    /// its age regardless of how often it is read — a document read a moment
    /// ago is no more playable than one read an hour ago if both were probed at
    /// the same time — so the oldest entry is both the least useful and the
    /// closest to expiring anyway. No read-time bookkeeping needed for it.
    fn prune(&mut self) {
        self.entries
            .retain(|_, e| e.stored.elapsed() < METADATA_MAX_AGE);
        while self.entries.len() > CAPACITY {
            let victim = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.seq)
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    self.entries.remove(&k);
                }
                None => break,
            }
        }
    }
}

/// What the one lock-holding decision concluded.
enum Claim {
    /// A cached document young enough for this caller. Nothing to do.
    Hit(Document),
    /// Nobody is probing this URL; the caller must, and now owns the marker.
    Lead(InFlightGuard),
    /// Somebody else is already probing it; wait on their result.
    Follow(broadcast::Receiver<Result<Document, String>>),
}

/// Owns one `in_flight` registration and guarantees its removal.
///
/// The deadlock this prevents: a leader that panics, is cancelled, or returns
/// early would otherwise leave its URL marked as in flight forever, and every
/// later caller would register as a follower on a channel nobody will ever send
/// on. Because the removal is in `Drop`, it happens on the panic and cancel
/// paths too, and dropping the last `Sender` closes the channel, which wakes
/// every waiter with `RecvError::Closed` — a signal they treat as "the leader
/// vanished, claim it again", not as an error.
struct InFlightGuard {
    cache: SharedMetadataCache,
    url: String,
    tx: broadcast::Sender<Result<Document, String>>,
}

impl InFlightGuard {
    /// Hands the leader's outcome to everyone waiting on it.
    ///
    /// `send` fails only when there are no receivers, which is the ordinary
    /// case of an uncontended probe — hence the discard.
    fn publish(&self, result: Result<Document, String>) {
        let _ = self.tx.send(result);
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        // Lock scope: one map removal. No I/O, no await; runs during unwinding.
        let mut c = lock(&self.cache);
        c.in_flight.remove(&self.url);
    }
}

/// The single lock-holding step: read the maps and, if this caller must do the
/// work, register it as in flight before releasing.
///
/// Everything expensive happens after this returns. That is the whole reason it
/// is a separate function taking no closure — there is no way to accidentally
/// `.await` inside it.
fn claim(cache: &SharedMetadataCache, url: &str, max_age: Duration) -> Claim {
    // Lock scope: two map lookups and at most one insert. No I/O, no await.
    let mut c = lock(cache);

    if let Some(entry) = c.entries.get(url) {
        if entry.stored.elapsed() < max_age {
            return Claim::Hit(entry.doc.clone());
        }
        // Too old for *this* caller, but deliberately left in place: a caller
        // with a longer tolerance (metadata, not playback) can still use it,
        // and the re-probe about to happen will overwrite it.
    }

    if let Some(tx) = c.in_flight.get(url) {
        return Claim::Follow(tx.subscribe());
    }

    // Capacity 1: there is exactly one message per probe, and a receiver that
    // subscribed before it was sent still gets it after every sender is gone.
    let (tx, _rx) = broadcast::channel(1);
    c.in_flight.insert(url.to_string(), tx.clone());
    Claim::Lead(InFlightGuard {
        cache: cache.clone(),
        url: url.to_string(),
        tx,
    })
}

/// Records a successful probe.
///
/// Only successes are recorded. A failure — rate limiting, a dropped
/// connection, an age gate — is transient often enough that caching it would
/// make the user wait out a TTL to retry something that would work now, so the
/// next caller for that URL probes again immediately.
fn store(cache: &SharedMetadataCache, url: &str, doc: Document) {
    // Lock scope: one insert plus the prune it triggers. No I/O, no await.
    let mut c = lock(cache);
    let seq = c.next_seq;
    c.next_seq += 1;
    c.entries.insert(
        url.to_string(),
        Entry {
            doc,
            stored: Instant::now(),
            seq,
        },
    );
    c.prune();
}

/// Returns the `--dump-single-json` document for `url`, running `probe` only if
/// no other caller has already produced or is already producing one.
///
/// `max_age` is how old a cached document this particular caller will accept:
/// `PLAYBACK_MAX_AGE` when the format URLs will be played, `METADATA_MAX_AGE`
/// when only the title and thumbnail are read. One cache, two tolerances.
///
/// `probe` is a `FnOnce` returning a future rather than the value directly so
/// that it is never *built* on a cache hit — the real one shells out through
/// `spawn_blocking`, and the point of a hit is that none of that happens.
///
/// A follower receives the leader's error as its own. It asked while the
/// leader's probe was in flight, so it would only be re-running a request that
/// just failed; the *next* call after that starts a fresh probe, since nothing
/// was cached.
pub async fn get_or_probe<F, Fut>(
    cache: &SharedMetadataCache,
    url: &str,
    max_age: Duration,
    probe: F,
) -> Result<Document, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<serde_json::Value, String>>,
{
    // `Option` only because the loop below cannot prove to the compiler that
    // the `FnOnce` is consumed at most once. It is: the branch that consumes it
    // returns.
    let mut probe = Some(probe);

    loop {
        match claim(cache, url, max_age) {
            Claim::Hit(doc) => return Ok(doc),

            Claim::Follow(mut rx) => match rx.recv().await {
                Ok(result) => return result,
                // The leader vanished without publishing — it panicked or was
                // cancelled. Its guard has already cleared the marker, so going
                // round again either finds a fresh entry or takes the lead.
                Err(_) => continue,
            },

            Claim::Lead(guard) => {
                let run = probe
                    .take()
                    .expect("the leading branch runs at most once per call");
                // Deliberately outside every lock: this is the 35-second part.
                // `guard` holds only the in-flight marker, not the mutex.
                let outcome = run().await;

                let published = match outcome {
                    Ok(value) => {
                        let doc: Document = Arc::new(value);
                        store(cache, url, doc.clone());
                        Ok(doc)
                    }
                    Err(e) => Err(e),
                };
                guard.publish(published.clone());
                return published;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A stand-in for `dump_single_json` that counts how often it actually ran.
    /// Every test here asserts on that count rather than on timing.
    fn counting_probe(
        calls: &Arc<AtomicUsize>,
        result: Result<serde_json::Value, String>,
    ) -> impl FnOnce() -> std::future::Ready<Result<serde_json::Value, String>> {
        let calls = calls.clone();
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(result)
        }
    }

    #[tokio::test]
    async fn a_fresh_entry_is_served_without_running_the_probe() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        let first = get_or_probe(
            &cache,
            "u",
            PLAYBACK_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "one" }))),
        )
        .await
        .expect("the first call probes");
        assert_eq!(first["title"], "one");

        // A second probe would return a *different* document, so serving "one"
        // again proves the closure never ran.
        let second = get_or_probe(
            &cache,
            "u",
            PLAYBACK_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "two" }))),
        )
        .await
        .expect("the second call hits");
        assert_eq!(second["title"], "one");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// The whole reason both call sites take the same cache: the trim panel
    /// probes, then the enqueue asks for the same URL and must pay nothing.
    #[tokio::test]
    async fn the_playback_and_metadata_tolerances_share_one_entry() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        get_or_probe(
            &cache,
            "u",
            PLAYBACK_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "one" }))),
        )
        .await
        .expect("preview probes");

        let meta = get_or_probe(
            &cache,
            "u",
            METADATA_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "two" }))),
        )
        .await
        .expect("enqueue hits");

        assert_eq!(meta["title"], "one");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Zero tolerance stands in for an expired entry: `elapsed() < 0` is never
    /// true, so the stored document is refused exactly as an ageing one is.
    #[tokio::test]
    async fn an_entry_too_old_for_the_caller_is_probed_again() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        get_or_probe(
            &cache,
            "u",
            PLAYBACK_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "one" }))),
        )
        .await
        .expect("first probe");

        let refreshed = get_or_probe(
            &cache,
            "u",
            Duration::ZERO,
            counting_probe(&calls, Ok(json!({ "title": "two" }))),
        )
        .await
        .expect("second probe");

        assert_eq!(refreshed["title"], "two");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // And the refreshed document replaced the stale one rather than
        // accumulating beside it.
        assert_eq!(lock(&cache).entries.len(), 1);
    }

    /// A caller that tolerates an older document is not forced to re-probe just
    /// because a stricter caller found the same entry too old.
    #[tokio::test]
    async fn a_lenient_caller_still_uses_an_entry_the_strict_one_rejected() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        get_or_probe(
            &cache,
            "u",
            METADATA_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "one" }))),
        )
        .await
        .expect("first probe");

        // Strict caller refuses it and re-probes.
        get_or_probe(
            &cache,
            "u",
            Duration::ZERO,
            counting_probe(&calls, Ok(json!({ "title": "two" }))),
        )
        .await
        .expect("re-probe");

        // Lenient caller is served without a third probe.
        get_or_probe(
            &cache,
            "u",
            METADATA_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "three" }))),
        )
        .await
        .expect("hit");

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_probe_is_not_cached_and_retries_immediately() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        let failed = get_or_probe(
            &cache,
            "u",
            PLAYBACK_MAX_AGE,
            counting_probe(&calls, Err("Sign in to confirm you're not a bot".into())),
        )
        .await;
        assert!(failed.is_err());
        assert!(lock(&cache).entries.is_empty());
        // No leftover marker either, or the retry below would wait forever.
        assert!(lock(&cache).in_flight.is_empty());

        let retried = get_or_probe(
            &cache,
            "u",
            PLAYBACK_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "one" }))),
        )
        .await
        .expect("the retry is allowed through at once");
        assert_eq!(retried["title"], "one");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_requests_for_one_url_run_exactly_one_probe() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        let slow = |calls: Arc<AtomicUsize>| {
            move || async move {
                calls.fetch_add(1, Ordering::SeqCst);
                // Long enough that the second caller is certainly waiting by
                // the time this resolves, standing in for the 35 seconds the
                // real subprocess takes.
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(json!({ "title": "one" }))
            }
        };

        let a = get_or_probe(&cache, "u", PLAYBACK_MAX_AGE, slow(calls.clone()));
        let b = get_or_probe(&cache, "u", PLAYBACK_MAX_AGE, slow(calls.clone()));
        let (ra, rb) = tokio::join!(a, b);

        assert_eq!(ra.expect("leader")["title"], "one");
        assert_eq!(rb.expect("follower")["title"], "one");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Coalescing is per URL, not global: a second URL must not be made to wait
    /// behind the first.
    #[tokio::test]
    async fn different_urls_do_not_coalesce() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        let a = get_or_probe(
            &cache,
            "a",
            PLAYBACK_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "a" }))),
        );
        let b = get_or_probe(
            &cache,
            "b",
            PLAYBACK_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "b" }))),
        );
        let (ra, rb) = tokio::join!(a, b);

        assert_eq!(ra.expect("a")["title"], "a");
        assert_eq!(rb.expect("b")["title"], "b");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// A follower gets told what the leader was told rather than serially
    /// repeating a request that just failed.
    #[tokio::test]
    async fn a_follower_receives_the_leaders_failure() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        let failing = |calls: Arc<AtomicUsize>| {
            move || async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                Err::<serde_json::Value, String>("HTTP Error 429".into())
            }
        };

        let a = get_or_probe(&cache, "u", PLAYBACK_MAX_AGE, failing(calls.clone()));
        let b = get_or_probe(&cache, "u", PLAYBACK_MAX_AGE, failing(calls.clone()));
        let (ra, rb) = tokio::join!(a, b);

        assert!(ra.is_err());
        assert!(rb.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // And neither poisoned the cache for the next attempt.
        assert!(lock(&cache).entries.is_empty());
        assert!(lock(&cache).in_flight.is_empty());
    }

    /// The panic path is the one that would deadlock a naive in-flight map:
    /// the leader never publishes, so the waiter must be released and allowed
    /// to take the lead itself.
    #[tokio::test]
    async fn a_panicking_leader_releases_its_waiters() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        // The panic is contained in a task of its own; the waiter runs here.
        let panicking = {
            let cache = cache.clone();
            tokio::spawn(async move {
                get_or_probe(&cache, "u", PLAYBACK_MAX_AGE, || async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    panic!("extractor exploded");
                })
                .await
            })
        };

        // Give the leader time to register before following it.
        tokio::time::sleep(Duration::from_millis(5)).await;

        let waiter = get_or_probe(
            &cache,
            "u",
            PLAYBACK_MAX_AGE,
            counting_probe(&calls, Ok(json!({ "title": "one" }))),
        );

        let (leader, followed) = tokio::join!(panicking, waiter);
        assert!(leader.is_err(), "the leading task panicked");
        assert_eq!(
            followed.expect("the waiter was released and probed itself")["title"],
            "one"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(lock(&cache).in_flight.is_empty());
    }

    #[tokio::test]
    async fn the_entry_count_is_bounded_and_evicts_the_oldest() {
        let cache = new_shared();
        let calls = Arc::new(AtomicUsize::new(0));

        for i in 0..(CAPACITY + 4) {
            get_or_probe(
                &cache,
                &format!("u{i}"),
                PLAYBACK_MAX_AGE,
                counting_probe(&calls, Ok(json!({ "title": i }))),
            )
            .await
            .expect("probe");
        }

        let c = lock(&cache);
        assert_eq!(c.entries.len(), CAPACITY);
        // The four oldest went; the newest stayed.
        for i in 0..4 {
            assert!(!c.entries.contains_key(&format!("u{i}")), "u{i} evicted");
        }
        for i in 4..(CAPACITY + 4) {
            assert!(c.entries.contains_key(&format!("u{i}")), "u{i} retained");
        }
    }
}
