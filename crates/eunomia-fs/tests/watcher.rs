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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eunomia_fs::watcher::{TargetWatcher, WatchEvent};

/// The test debounce: short enough to keep the suite quick, a real
/// window the backend actually debounces inside.
const DEBOUNCE: Duration = Duration::from_millis(40);

/// How long a positive assertion waits for a delivery (generous for a
/// loaded runner; delivery is late, never absent).
const DELIVERY_DEADLINE: Duration = Duration::from_secs(10);

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
