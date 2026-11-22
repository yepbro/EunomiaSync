//! Integration tests of the target-file watcher (spec 5.7, SYS-007;
//! T-102): a real `notify` backend over real temporary fixtures, the
//! same mechanics the app service will drive — a file set, external
//! edits around it, and the debounced deliveries that come back.
//!
//! Timing: the debounce window is the test's own (40 ms — SYS-007's
//! 300 ms value is the production knob the app layer pins); positive
//! assertions poll a generous deadline (a slow CI runner delays
//! delivery, never cancels it), and "nothing happens" assertions wait
//! a quiet window of many debounce periods before believing the
//! silence. Every test asserts on the events AS DELIVERED — the
//! classification (Changed/Removed) is this module's contract; what
//! the events mean for project state is the app service's concern.
//!
//! The SYS-010 watch limit (T-103) has its own battery below: no
//! fixture can make a healthy EXISTING directory refuse a watch on
//! demand (the kernel refuses only once
//! `fs.inotify.max_user_watches` is genuinely exhausted), so those
//! tests run the real convergence against an injected backend with
//! a controlled refusal — the `WatchBackend` seam, D-137's P3-7.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eunomia_fs::watcher::{TargetWatcher, WatchBackend, WatchEvent, WatchRefused};

/// The test debounce: short enough to keep the suite quick, a real
/// window the backend actually debounces inside.
const DEBOUNCE: Duration = Duration::from_millis(40);

/// How long a positive assertion waits for a delivery (generous for a
/// loaded runner; delivery is late, never absent).
const DELIVERY_DEADLINE: Duration = Duration::from_secs(60);

/// How long a negative assertion waits before believing silence
/// (many debounce periods — events the backend will deliver arrive
/// far inside it).
const QUIET_WINDOW: Duration = Duration::from_millis(800);

/// A watcher with its event receiver, the 40 ms test window.
fn watcher() -> (TargetWatcher, Receiver<WatchEvent>) {
    let (sender, receiver) = mpsc::channel();
    let watcher = TargetWatcher::new(DEBOUNCE, sender).expect("constructing the watcher must work");
    (watcher, receiver)
}

/// Collects everything that arrives within `wait`, panicking on a
/// backend error (a `Backend` event is a watcher failure the suite
/// must not paper over by ignoring it).
fn events_within(receiver: &Receiver<WatchEvent>, wait: Duration) -> Vec<WatchEvent> {
    let deadline = Instant::now() + wait;
    let mut collected = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return collected;
        }
        match receiver.recv_timeout(remaining) {
            Ok(WatchEvent::Backend) => {
                panic!("the watcher backend reported an error mid-test");
            }
            Ok(event) => collected.push(event),
            Err(mpsc::RecvTimeoutError::Timeout) => return collected,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the watcher died mid-test");
            }
        }
    }
}

/// Waits for one event matching `predicate` and returns it, failing
/// with everything that arrived instead if the deadline passes — the
/// positive-assertion shape that RETURNS as soon as the event lands
/// (a slow runner delays delivery, never cancels it, so the generous
/// ceiling is kept while the wall time is not burned).
fn expect_event(
    receiver: &Receiver<WatchEvent>,
    wait: Duration,
    predicate: impl Fn(&WatchEvent) -> bool,
) -> WatchEvent {
    let deadline = Instant::now() + wait;
    let mut seen = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("no matching event within {wait:?}, saw {seen:?}");
        }
        match receiver.recv_timeout(remaining) {
            Ok(WatchEvent::Backend) => {
                panic!("the watcher backend reported an error mid-test");
            }
            Ok(event) => {
                if predicate(&event) {
                    return event;
                }
                seen.push(event);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                panic!("no matching event within {wait:?}, saw {seen:?}");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the watcher died mid-test");
            }
        }
    }
}

/// Waits until no event matching `predicate` arrives within `wait` —
/// the negative-assertion form of [`expect_event`]: it keeps
/// listening the whole window and reports what actually matched.
fn expect_no_event(
    receiver: &Receiver<WatchEvent>,
    wait: Duration,
    predicate: impl Fn(&WatchEvent) -> bool + Copy,
) {
    let matched: Vec<WatchEvent> = events_within(receiver, wait)
        .into_iter()
        .filter(predicate)
        .collect();
    assert!(
        matched.is_empty(),
        "no matching events expected, got {matched:?}"
    );
}

/// Asserts nothing arrives within the quiet window — the honest form
/// of "no event": collect, then fail with what actually came.
fn assert_quiet(receiver: &Receiver<WatchEvent>) {
    let stray = events_within(receiver, QUIET_WINDOW);
    assert!(stray.is_empty(), "no events expected, got {stray:?}");
}

/// A one-file set over an existing file.
fn one_file(path: &Path) -> Vec<PathBuf> {
    vec![path.to_path_buf()]
}

/// Writes a file with content, creating parents as needed.
fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("creating the parent must work");
    }
    std::fs::write(path, content).expect("writing the fixture file must work");
}

// ---- the deliveries ----------------------------------------------------

#[test]
fn an_external_edit_of_a_watched_file_delivers_changed() {
    // The base contract (SYS-008's trigger): an edit made outside the
    // app — a plain std write, exactly what an editor does — reaches
    // the consumer as one Changed event for that file.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let target = temp.path().join("AGENTS.md");
    write(&target, "one");
    let (mut watcher, receiver) = watcher();
    watcher.set_files(&one_file(&target));
    assert_eq!(watcher.watched_files(), set_of(&one_file(&target)));

    write(&target, "an external edit");
    expect_event(&receiver, DELIVERY_DEADLINE, |event| {
        *event == WatchEvent::Changed(target.clone())
    });
    expect_no_event(&receiver, QUIET_WINDOW, |event| {
        matches!(event, WatchEvent::Removed(_))
    });
}

#[test]
fn an_atomic_rename_over_the_target_delivers_changed() {
    // The inode-trap pin, the reason the watch sits on the parent
    // directory (module docs): a save that stages a temp file and
    // renames it over the target — this app's own §9.5 write shape
    // and every atomic editor save — must still be seen. A watch
    // pinned to the old inode would go silent exactly here.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let target = temp.path().join("AGENTS.md");
    write(&target, "old");
    let (mut watcher, receiver) = watcher();
    watcher.set_files(&one_file(&target));

    let staged = temp.path().join(".AGENTS.md.eunomiasync-new");
    write(&staged, "the new bytes");
    std::fs::rename(&staged, &target).expect("the atomic replace must work");
    expect_event(&receiver, DELIVERY_DEADLINE, |event| {
        *event == WatchEvent::Changed(target.clone())
    });
}

#[test]
fn deleting_a_watched_file_delivers_removed() {
    // SYS-009's trigger: the external deletion is delivered as a
    // removal of the name.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let target = temp.path().join("AGENTS.md");
    write(&target, "doomed");
    let (mut watcher, receiver) = watcher();
    watcher.set_files(&one_file(&target));

    std::fs::remove_file(&target).expect("the deletion must work");
    expect_event(&receiver, DELIVERY_DEADLINE, |event| {
        *event == WatchEvent::Removed(target.clone())
    });
}

#[test]
fn renaming_a_watched_file_away_delivers_removed() {
    // The other way a name stops existing: the file moves elsewhere.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let target = temp.path().join("AGENTS.md");
    write(&target, "moving");
    let (mut watcher, receiver) = watcher();
    watcher.set_files(&one_file(&target));

    std::fs::rename(&target, temp.path().join("elsewhere.md")).expect("the rename must work");
    expect_event(&receiver, DELIVERY_DEADLINE, |event| {
        *event == WatchEvent::Removed(target.clone())
    });
}

// ---- the set semantics --------------------------------------------------

#[test]
fn a_sibling_file_in_a_watched_directory_is_not_reported() {
    // "Only the target files are watched": the directory watch is the
    // mechanism, the NAME filter is the contract — a neighboring file
    // the set never named must stay invisible.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let target = temp.path().join("AGENTS.md");
    let sibling = temp.path().join("README.md");
    write(&target, "watched");
    write(&sibling, "not watched");
    let (mut watcher, receiver) = watcher();
    watcher.set_files(&one_file(&target));

    write(&sibling, "a sibling edit");
    write(&target, "and a real edit to flush the window");
    let events = events_within(&receiver, DELIVERY_DEADLINE);
    assert!(
        !events.iter().any(|event| matches!(event, WatchEvent::Changed(p) | WatchEvent::Removed(p) if p == &sibling)),
        "the sibling's edit must not be reported: {events:?}"
    );
    assert!(
        events.contains(&WatchEvent::Changed(target.clone())),
        "the target's edit must be reported alongside: {events:?}"
    );
}

#[test]
fn dropping_a_file_from_the_set_stops_its_events_and_readding_restores_them() {
    // SYS-007's removal half: a file that leaves the set (archived,
    // unavailable) stops being observed, and a later return is
    // observed again — the full-set update is a real convergence, not
    // an append.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let target = temp.path().join("AGENTS.md");
    write(&target, "one");
    let (mut watcher, receiver) = watcher();
    watcher.set_files(&one_file(&target));

    watcher.set_files(&[]);
    assert!(
        watcher.watched_files().is_empty(),
        "an empty set converges to no files"
    );
    write(&target, "unwatched now");
    assert_quiet(&receiver);

    watcher.set_files(&one_file(&target));
    assert_eq!(watcher.watched_files(), set_of(&one_file(&target)));
    write(&target, "watched again");
    expect_event(&receiver, DELIVERY_DEADLINE, |event| {
        *event == WatchEvent::Changed(target.clone())
    });
}

#[test]
fn a_repeated_identical_set_is_idempotent() {
    // principle: make operations idempotent. Setting the same set
    // twice must not duplicate watches (the converged set is stable)
    // nor double-deliver (one edit → its Changed events, nothing
    // synthetic from the set operation itself).
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let target = temp.path().join("AGENTS.md");
    write(&target, "one");
    let (mut watcher, receiver) = watcher();
    watcher.set_files(&one_file(&target));
    // The first set may race real filesystem noise from the fixture
    // write; flush it so the pin below starts from a quiet channel.
    let _ = events_within(&receiver, QUIET_WINDOW);

    watcher.set_files(&one_file(&target));
    assert_eq!(watcher.watched_files(), set_of(&one_file(&target)));
    assert_quiet(&receiver);

    write(&target, "still exactly one logical watch");
    expect_event(&receiver, DELIVERY_DEADLINE, |event| {
        *event == WatchEvent::Changed(target.clone())
    });
    // Settle the debounce window and count the duplicates: one std
    // write may surface as at most two backend events, never a storm.
    let late = events_within(&receiver, QUIET_WINDOW);
    let delivered = 1 + late
        .iter()
        .filter(|event| **event == WatchEvent::Changed(target.clone()))
        .count();
    assert!(
        delivered <= 2,
        "one std write must not fan out into a storm: {delivered} Changed events"
    );
}

#[test]
fn a_file_under_a_missing_directory_converges_to_unwatched_without_an_error() {
    // The tolerance rule: an unavailable root means NO watch, never
    // an error (SYS-007's removal half, the module docs' "the map is
    // the truth"). The same file watches fine once its directory
    // exists and the set is set again — convergence is recoverable.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let target = temp.path().join("unborn").join("AGENTS.md");
    let (mut watcher, receiver) = watcher();

    watcher.set_files(&one_file(&target));
    assert!(
        watcher.watched_files().is_empty(),
        "a file under a missing directory cannot be watched: {:#?}",
        watcher.watched_files()
    );
    assert_quiet(&receiver);

    write(&target, "the directory exists now");
    watcher.set_files(&one_file(&target));
    assert_eq!(
        watcher.watched_files(),
        set_of(&one_file(&target)),
        "the next set_files picks the now-existing directory up"
    );
    write(&target, "an edit the recovered watch must see");
    expect_event(&receiver, DELIVERY_DEADLINE, |event| {
        *event == WatchEvent::Changed(target.clone())
    });
}

#[test]
fn two_files_in_one_directory_are_both_served() {
    // The dedup is the watch BUDGET's (one backend watch per
    // directory), never the events': each file keeps its own
    // deliveries (§5.7's scheme-C pair shape: two target files in one
    // repository root).
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let agents = temp.path().join("AGENTS.md");
    let stem = temp.path().join("AGENTS.project.md");
    write(&agents, "base");
    write(&stem, "stem");
    let both = vec![agents.clone(), stem.clone()];
    let (mut watcher, receiver) = watcher();
    watcher.set_files(&both);
    assert_eq!(watcher.watched_files(), set_of(&both));

    write(&stem, "the stem file is edited");
    let events = events_within(&receiver, DELIVERY_DEADLINE);
    assert!(
        events.contains(&WatchEvent::Changed(stem.clone())),
        "the stem file's edit is delivered: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, WatchEvent::Changed(p) if p == &agents)),
        "the untouched base file has no event: {events:?}"
    );
}

/// The set-of helper, for stable comparisons.
fn set_of(paths: &[PathBuf]) -> BTreeSet<PathBuf> {
    paths.iter().cloned().collect()
}

// ---- the SYS-010 seam and the re-arm ------------------------------------

#[test]
fn a_refused_watch_is_reported_through_the_unwatched_seam() {
    // SYS-010's observable: a wanted file whose directory the backend
    // refuses to watch is not an error and not an event — it is the
    // unwatched set, the seam the app layer's warning line and
    // non-blocking notice read. Empty for a healthy converged set,
    // the exact missing files otherwise, cleared by the next
    // successful convergence.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let target = temp.path().join("unborn").join("AGENTS.md");
    let (mut watcher, _receiver) = watcher();

    watcher.set_files(&one_file(&target));
    assert!(
        watcher.watched_files().is_empty(),
        "a file under a missing directory cannot be watched"
    );
    assert_eq!(
        watcher.unwatched_files(),
        set_of(&one_file(&target)),
        "the refused watch is reported as wanted-but-unwatched, not \
         swallowed"
    );

    write(&target, "the directory exists now");
    watcher.set_files(&one_file(&target));
    assert_eq!(watcher.watched_files(), set_of(&one_file(&target)));
    assert!(
        watcher.unwatched_files().is_empty(),
        "a converged set has no gap to report"
    );
}

#[test]
fn a_recreated_directory_is_re_armed_by_the_next_set() {
    // The kernel ends a directory's watch with a silent IN_IGNORED
    // when the directory is deleted; set_files re-arms every wanted
    // directory unconditionally (idempotent for a live watch), so the
    // deleted-and-recreated root is healed by the next full pass
    // without the caller having observed the loss. The map alone
    // ("already known") would keep the dead watch forever.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let dir = temp.path().join("root");
    let target = dir.join("AGENTS.md");
    write(&target, "one");
    let (mut watcher, receiver) = watcher();
    watcher.set_files(&one_file(&target));
    assert_eq!(watcher.watched_files(), set_of(&one_file(&target)));

    std::fs::remove_dir_all(&dir).expect("removing the directory must work");
    write(&target, "recreated"); // the parent is created again — a NEW inode
    watcher.set_files(&one_file(&target)); // the re-arm the test pins
    write(&target, "an edit the healed watch must see");
    expect_event(&receiver, DELIVERY_DEADLINE, |event| {
        *event == WatchEvent::Changed(target.clone())
    });
}

// ---- the SYS-010 watch limit (the injected backend) --------------------

/// The shared plan: refuse `watch` from a 1-based call number on
/// (`None` = the lifted limit, arm everything).
type LimitPlan = Arc<Mutex<Option<usize>>>;

/// The shared re-arm log: every arm call's directory, in call order.
type WatchLog = Arc<Mutex<Vec<PathBuf>>>;

/// The injected backend of the watch-limit battery (the
/// `WatchBackend` seam, D-137's P3-7): every arm call is logged in
/// order, and from a chosen 1-based call number on, arming is
/// REFUSED — the `fs.inotify.max_user_watches` shape, where the
/// kernel refuses an EXISTING healthy directory and no fixture can.
/// The plan is shared with the test so the limit can be lifted with
/// the watcher already built; a refusal is persistent until the plan
/// says otherwise, which is what a kernel limit does.
struct LimitBackend {
    /// The shared plan (see [`LimitPlan`]).
    limit_from: LimitPlan,
    /// The shared re-arm log (see [`WatchLog`]) — what the
    /// convergence battery asserts on.
    watch_calls: WatchLog,
}

impl LimitBackend {
    /// The backend refusing from the `refuse_from`-th arm call on,
    /// plus the handles the test keeps (the plan to lift, the log
    /// to read).
    fn refusing_from(refuse_from: usize) -> (Self, LimitPlan, WatchLog) {
        let limit_from: LimitPlan = Arc::new(Mutex::new(Some(refuse_from)));
        let watch_calls: WatchLog = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                limit_from: Arc::clone(&limit_from),
                watch_calls: Arc::clone(&watch_calls),
            },
            limit_from,
            watch_calls,
        )
    }
}

impl WatchBackend for LimitBackend {
    fn watch(&mut self, dir: &Path) -> Result<(), WatchRefused> {
        let call_number = {
            let mut log = self
                .watch_calls
                .lock()
                .expect("only the watcher calls this backend");
            log.push(dir.to_path_buf());
            log.len()
        };
        let limit = self
            .limit_from
            .lock()
            .expect("only the watcher calls this backend");
        if limit.is_some_and(|from| call_number >= from) {
            // The reason names the class, never a raw OS errno: the
            // production refusal arrives as notify's MaxFilesWatch
            // (Linux maps the watches limit's ENOSPC to it; the
            // instances limit's EMFILE stays a plain I/O error), and
            // the convergence under test never branches on the
            // reason — a platform-neutral test states no errno
            // (D-111).
            Err(WatchRefused::new(
                "the synthetic inotify watch limit (SYS-010)".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    fn unwatch(&mut self, _dir: &Path) {
        // The battery never removes a directory from the set; the
        // disarm half of the convergence is the real backend's
        // tests' concern above.
    }
}

#[test]
fn a_watch_limit_hit_mid_set_leaves_the_later_directories_unwatched() {
    // SYS-010's refused-watch half at its least reachable point:
    // the limit refuses an EXISTING healthy directory mid-set (beta
    // onward; the wanted map's BTreeMap order is alpha, beta,
    // gamma). The honest degrade: the later directories are
    // unwatched — exactly them, through the seam — while the
    // already-converged directory STAYS converged: its re-arm is
    // refused too, and the map keeps the evidence it has. (A
    // genuinely LIVE watch's re-arm cannot be refused by the
    // watches limit — the kernel updates the existing mark without
    // allocating — so the refused re-arm of a converged directory
    // stands for the lost-watch-plus-limit shape: the backend lost
    // the watch behind the caller's back and the limit blocks the
    // re-add.) The same set again answers identically — idempotent
    // under the limit.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let (_alpha_dir, alpha) = target_under(&temp, "alpha");
    let (_beta_dir, beta) = target_under(&temp, "beta");
    let (_gamma_dir, gamma) = target_under(&temp, "gamma");
    let all = vec![alpha.clone(), beta.clone(), gamma.clone()];
    let (backend, _limit_from, _watch_calls) = LimitBackend::refusing_from(2);
    let mut watcher = TargetWatcher::with_backend(backend);

    watcher.set_files(&all);
    assert_eq!(
        watcher.watched_files(),
        set_of(std::slice::from_ref(&alpha)),
        "the directories before the limit hit keep their watches"
    );
    assert_eq!(
        watcher.unwatched_files(),
        set_of(&[beta.clone(), gamma.clone()]),
        "the seam answers exactly the refused directories' files"
    );

    watcher.set_files(&all);
    assert_eq!(
        watcher.watched_files(),
        set_of(std::slice::from_ref(&alpha)),
        "the already-converged directory survives its refused re-arm"
    );
    assert_eq!(
        watcher.unwatched_files(),
        set_of(&[beta.clone(), gamma.clone()]),
        "the same set under the same limit answers identically"
    );
}

#[test]
fn lifting_the_watch_limit_converges_the_refused_directories_on_the_next_set() {
    // The retry the seam promises, with no flag and no error
    // bookkeeping: the re-arm is unconditional, so the first
    // set_files after the limit lifts simply succeeds — the whole
    // set converges and the seam empties.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let (_alpha_dir, alpha) = target_under(&temp, "alpha");
    let (_beta_dir, beta) = target_under(&temp, "beta");
    let (_gamma_dir, gamma) = target_under(&temp, "gamma");
    let all = vec![alpha.clone(), beta.clone(), gamma.clone()];
    let (backend, limit_from, _watch_calls) = LimitBackend::refusing_from(2);
    let mut watcher = TargetWatcher::with_backend(backend);

    watcher.set_files(&all);
    assert_eq!(
        watcher.unwatched_files(),
        set_of(&[beta.clone(), gamma.clone()])
    );

    *limit_from
        .lock()
        .expect("the plan handle is the test's own") = None;
    watcher.set_files(&all);
    assert_eq!(watcher.watched_files(), set_of(&all));
    assert!(
        watcher.unwatched_files().is_empty(),
        "a converged set has no gap to report: {:#?}",
        watcher.unwatched_files()
    );
}

#[test]
fn every_pass_re_arms_every_wanted_directory_refused_or_not() {
    // The convergence's unconditional re-arm — the documented rule
    // that heals a kernel-lost watch — pinned through the injected
    // backend's log: every set_files pass arms every wanted
    // directory, the refused ones included (that IS the idempotent
    // retry the seam's recovery rides on), in the wanted map's
    // order. A convergence that trusted "already in the map" would
    // log the first pass only.
    let temp = tempfile::tempdir().expect("creating a temp directory must work");
    let (alpha_dir, alpha) = target_under(&temp, "alpha");
    let (beta_dir, beta) = target_under(&temp, "beta");
    let (gamma_dir, gamma) = target_under(&temp, "gamma");
    let all = vec![alpha, beta, gamma];
    let (backend, _limit_from, watch_calls) = LimitBackend::refusing_from(2);
    let mut watcher = TargetWatcher::with_backend(backend);

    watcher.set_files(&all);
    watcher.set_files(&all);
    let logged = watch_calls
        .lock()
        .expect("the log handle is the test's own")
        .clone();
    assert_eq!(
        logged,
        vec![
            alpha_dir.clone(),
            beta_dir.clone(),
            gamma_dir.clone(),
            alpha_dir,
            beta_dir,
            gamma_dir,
        ],
        "two passes each arm every wanted directory, refused ones included"
    );
}

/// A written target file under a freshly created `<name>`
/// subdirectory of `temp`, with the directory's path alongside —
/// the fixture keeps the refused directories REAL (the limit
/// refuses healthy directories; a missing-root refusal is a
/// different, already-tested tolerance).
fn target_under(temp: &tempfile::TempDir, name: &str) -> (PathBuf, PathBuf) {
    let dir = temp.path().join(name);
    let target = dir.join("AGENTS.md");
    write(&target, "fixture");
    (dir, target)
}
