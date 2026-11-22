//! Target-file watching (spec 5.7, SYS-007/SYS-009; DECISIONS.md D-004):
//! the debounced file-system watcher over exactly the set of target
//! files the caller hands in — one logical watch per file, no tree
//! walks, no other responsibility.
//!
//! The module is deliberately mechanic, not semantic: it knows file
//! paths and events, nothing about projects, builds or schemes. The
//! app-layer service (`src-tauri/src/file_watch.rs`) decides WHICH
//! files make up the set (active + available projects, §5.7's closed
//! name list) and what an event MEANS (the SYS-008 re-read and
//! comparison); this crate only keeps the watches and the debounce
//! honest. That split keeps the file-system half testable without a
//! database and without Tauri (the crate's standing boundary, D-010).
//!
//! # The watch sits on the parent directory, one per directory
//!
//! SYS-007 words the mechanics "one watch per file", and the watcher
//! honors that as a PER-FILE guarantee: every file in the set is
//! observed individually (its events, and only its events, are
//! delivered), no file is watched twice, and no directory tree is
//! ever walked recursively. The underlying `notify` watch is placed
//! on the file's PARENT directory, non-recursively, because the
//! inotify backend watches inodes: this app's own atomic writes
//! (§9.5 write-then-rename) and every editor's atomic save replace
//! the target with a NEW inode, and a watch pinned to the old inode
//! would silently stop seeing the file — exactly the external edits
//! this watcher exists to catch. A directory watch survives the
//! rename by construction. Directories are deduplicated: two target
//! files in one repository root share one watch, which is also the
//! kind posture toward `fs.inotify.max_user_watches` (SYS-010's
//! budget).
//!
//! # The converged set and the tolerated root
//!
//! [`TargetWatcher::set_files`] is a full-set operation, the
//! idempotent way in: it converges the live watches to exactly the
//! wanted files (adds the new directories, drops the emptied ones,
//! keeps the rest), however many times it runs and whatever the
//! previous set was. A directory that CANNOT be watched — the root is
//! gone, a dead drive, permissions — is not an error: the file simply
//! leaves the converged set [`TargetWatcher::watched_files`]
//! answers, and the next `set_files` retries it (an unavailable
//! project has no watch, per SYS-007's removal rule; the availability
//! gate itself is the caller's). Nothing panics and nothing is
//! watched half-way: the map is the truth.
//!
//! # Events
//!
//! All deliveries pass through `notify-debouncer-full`'s debounce
//! window (SYS-007's 300 ms in production; a caller-chosen
//! [`Duration`], so tests run a short window) and land on the
//! caller's [`std::sync::mpsc::Sender`] as [`WatchEvent`]s — one per
//! affected watched file: [`WatchEvent::Changed`] for anything that
//! created, modified or renamed onto the name, [`WatchEvent::Removed`]
//! for a removal or a rename-away of it. `notify` access-only noise
//! (reads) is dropped. The backend's LOST-EVENTS signals arrive as
//! [`WatchEvent::Backend`] — one event per debouncer batch: the
//! event-queue overflow (SYS-010's case, which inotify delivers as
//! a rescan flag riding an ordinary batch, with no paths of its
//! own) and a stream error batch alike. The caller's documented
//! response is to re-check every target file once, and the logging
//! of the failure belongs to the app layer's facade. A REFUSED
//! WATCH (an unwatchable root, the inotify watches limit) is a
//! different, persistent fact: it never reaches the event stream —
//! the file simply stays out of the converged set, and the caller
//! reads the gap from [`TargetWatcher::unwatched_files`] instead
//! (turning a refusal into an event would loop: `Backend` →
//! re-check → `set_files` → `Backend`, with nothing new to say).
//! The classification is advisory — honest state comes from
//! re-reading the file — but it is tested as delivered.
//!
//! # The backend seam
//!
//! The two calls `set_files` makes of the watching backend — arm
//! and disarm one non-recursive directory watch — sit behind
//! [`WatchBackend`], with [`NotifyBackend`] (one
//! `notify-debouncer-full` debouncer) as the production arm. The
//! seam is D-137's recorded follow-up P3-7: the inotify watches
//! limit refuses an EXISTING, healthy directory, and no filesystem
//! fixture can reproduce that on demand — the kernel refuses only
//! once `fs.inotify.max_user_watches` is genuinely exhausted
//! (notify 8.2 surfaces it as `MaxFilesWatch` after the kernel's
//! watch-limit `ENOSPC`; the separate INSTANCES limit `EMFILE`
//! fails the watcher at CONSTRUCTION — `inotify_init` allocating
//! the descriptor — which is [`WatcherInitError`]'s
//! no-watch-at-all degrade, not a per-directory refusal: adding a
//! watch allocates no descriptor and cannot fail with `EMFILE`).
//! Tests substitute a backend with a controlled refusal and drive
//! the REAL convergence, while [`TargetWatcher::new`] stays the
//! unchanged production constructor and
//! [`TargetWatcher::with_backend`] is the seam's way in.
//!
//! # Testing plan
//!
//! Integration tests over real temporary fixtures (`tests/watcher.rs`):
//! an external edit and an atomic rename-over both deliver a Changed
//! event for the file; a deletion and a rename-away deliver Removed; a
//! sibling file in the same watched directory delivers nothing (the
//! name filter); dropping a file from the set stops its events and
//! re-adding restores them; a repeated identical `set_files` keeps
//! the converged set stable (idempotence); a file under a nonexistent
//! directory converges to unwatched without an error, and the same
//! file watches fine once the directory exists and the set is set
//! again; two files in one directory are both served (the dedup is
//! the watch budget's, not the events').
//!
//! The SYS-010 halves beyond that: the watch-limit battery runs
//! through [`TargetWatcher::with_backend`] with a refusing backend
//! — a limit hit mid-set leaves the later directories unwatched
//! (the unwatched seam answers exactly them) while the
//! already-converged directory keeps its convergence, a repeated
//! pass under the limit answers identically, lifting the limit
//! lets the very next `set_files` converge everything, and every
//! pass re-arms every wanted directory (the refused ones included
//! — the idempotent retry). The event-queue overflow half is
//! pinned at the extracted `dispatch` by constructing the batches
//! notify's public API allows (the overflow is a rescan-flag
//! `Ok` batch — pin: one `Backend` per batch, ordinary events
//! alongside it still delivered, and a mixed batch delivers both;
//! a stream error `Err` batch is one `Backend` too; a dead
//! consumer is not a panic).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc::Sender};
use std::time::Duration;

use notify_debouncer_full::notify::RecommendedWatcher;
use notify_debouncer_full::notify::RecursiveMode;
use notify_debouncer_full::notify::event::{EventKind, ModifyKind, RenameMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};

/// One debounced fact about one watched file (spec 5.7): the name was
/// changed (created, written, renamed over) or removed (deleted,
/// renamed away) — or the backend itself reported trouble.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// The file name now holds different (or new) bytes. The event is
    /// advisory: the honest check is reading the file.
    Changed(PathBuf),
    /// The file name no longer exists on disk (a removal or a
    /// rename-away).
    Removed(PathBuf),
    /// The watcher backend lost or cannot trust its event stream
    /// (SYS-010's event-queue overflow — delivered by inotify as a
    /// rescan flag riding an ordinary batch — or a stream error
    /// batch; a refused watch is NOT this, see
    /// [`TargetWatcher::unwatched_files`]). The failure's details
    /// belong to the app layer's logging; the sanctioned reaction
    /// here is one full re-check of every target file.
    Backend,
}

/// The failure of [`TargetWatcher::new`]: the debouncer and its
/// backend could not be constructed at all (a system watcher limit, a
/// resource error). Watching individual directories cannot fail this
/// way — [`TargetWatcher::set_files`] tolerates those (module docs).
/// Carries a class-only diagnostic string, never file content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatcherInitError(String);

impl fmt::Display for WatcherInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the file watcher backend failed to start: {}", self.0)
    }
}

impl std::error::Error for WatcherInitError {}

/// The refusal of one directory watch: the backend would not arm a
/// watch on the directory — on Linux the WATCHES limit
/// (`fs.inotify.max_user_watches`, surfaced by notify 8.2 as
/// `MaxFilesWatch` after the kernel's `ENOSPC`), elsewhere an
/// unwatchable or vanished root. (The separate INSTANCES limit
/// `EMFILE` never gets this far: `inotify_init` fails at watcher
/// CONSTRUCTION, which is [`WatcherInitError`]'s no-watch-at-all
/// degrade — adding a watch allocates no descriptor.) Carries a
/// class-only diagnostic string, never file content. The watcher's
/// response never branches on the reason: a refusal is a persistent
/// fact about the directory, answered by the converged set and
/// [`TargetWatcher::unwatched_files`], not an error and not an
/// event (module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRefused(String);

impl WatchRefused {
    /// Wraps the backend's refusal reason — its class text (an OS
    /// error's kind or code name), never file content.
    pub fn new(reason: String) -> Self {
        Self(reason)
    }
}

impl fmt::Display for WatchRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the file watcher backend refused the directory: {}",
            self.0
        )
    }
}

impl std::error::Error for WatchRefused {}

/// The watching backend [`TargetWatcher`] drives: the two calls
/// `set_files` makes — arm one non-recursive directory watch, disarm
/// one — nothing more. The production arm is [`NotifyBackend`] over
/// `notify-debouncer-full`; the seam (D-137's recorded follow-up
/// P3-7) exists so a test can refuse a watch on an EXISTING healthy
/// directory — the exact shape of the inotify watch limit
/// (SYS-010), which no filesystem fixture can reproduce on demand —
/// and observe the real convergence's answer to it.
///
/// The contract of [`WatchBackend::watch`]: arming a directory that
/// is already armed must behave like arming it fresh (the caller
/// re-arms unconditionally — the kernel can lose a watch behind its
/// back, so every wanted directory gets the syscall again on every
/// pass). A refusal is a fact until a later call succeeds; the
/// caller never retries within one `set_files` pass.
pub trait WatchBackend {
    /// Arms (re-arms) the non-recursive watch of `dir`.
    ///
    /// # Errors
    ///
    /// [`WatchRefused`] when the backend will not arm the watch —
    /// the watch limit, an unwatchable root, a dead volume. The
    /// caller's response is the converged set, not the error.
    fn watch(&mut self, dir: &Path) -> Result<(), WatchRefused>;

    /// Disarms the watch of `dir`. The caller treats the directory
    /// as forgotten either way: a failure here means the backend
    /// already dropped the watch (or the volume died), and the next
    /// `set_files` re-adds it cleanly — so there is nothing to
    /// report.
    fn unwatch(&mut self, dir: &Path);
}

/// The production [`WatchBackend`]: one `notify-debouncer-full`
/// debouncer (SYS-007's 300 ms window in production) whose event
/// thread runs the extracted `dispatch` over the shared watch map
/// and the caller's sink. Built by [`TargetWatcher::new`]; dropping
/// it stops the backend.
pub struct NotifyBackend {
    /// The debouncer guard; owns the backend thread and stops it on
    /// drop. `watch`/`unwatch` are the only calls made on it.
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

impl NotifyBackend {
    /// Builds the debouncer with the `debounce` window, wiring its
    /// event thread to `dirs` (the shared filter map) and `events`
    /// (the sink). The callback is a one-line forward into
    /// `dispatch` — the mapping itself lives there, extracted so
    /// both of its arms (deliveries and the overflow signal) are
    /// testable without notify's own event thread.
    fn new(
        debounce: Duration,
        dirs: WatchedDirs,
        events: Sender<WatchEvent>,
    ) -> Result<Self, WatcherInitError> {
        let filter_dirs = dirs;
        let sink = events;
        let debouncer = new_debouncer(debounce, None, move |result: DebounceEventResult| {
            dispatch(result, &filter_dirs, &sink);
        })
        .map_err(|error| WatcherInitError(error.to_string()))?;
        Ok(Self { debouncer })
    }
}

impl WatchBackend for NotifyBackend {
    fn watch(&mut self, dir: &Path) -> Result<(), WatchRefused> {
        self.debouncer
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|error| WatchRefused(error.to_string()))
    }

    fn unwatch(&mut self, dir: &Path) {
        let _ = self.debouncer.unwatch(dir);
    }
}

/// The live watch map shared with the debouncer's event thread: the
/// watched directories and, per directory, the exact file names the
/// events are filtered to. Behind a [`Mutex`] because the debouncer's
/// closure reads it from its own thread while `set_files` mutates it;
/// both critical sections are short and never block on I/O.
type WatchedDirs = Arc<Mutex<HashMap<PathBuf, BTreeSet<OsString>>>>;

/// The one lock message of this module's two short critical sections.
const LOCK_BROKEN: &str = "the watch map is locked only inside short synchronous sections";

/// A debounced watcher over a caller-defined set of target files
/// (spec 5.7, SYS-007): constructed once with the debounce window and
/// the event sink, then driven entirely through
/// [`TargetWatcher::set_files`] — the full-set, idempotent update.
///
/// Dropping the value stops the backend; the sender passed to
/// [`TargetWatcher::new`] sees the channel close.
///
/// Generic over the [`WatchBackend`] seam (module docs): the default
/// is the production [`NotifyBackend`], so plain `TargetWatcher`
/// everywhere means the real thing, and
/// [`TargetWatcher::with_backend`] is the test seam's way in.
pub struct TargetWatcher<B: WatchBackend = NotifyBackend> {
    /// The watching backend (the seam, module docs):
    /// [`NotifyBackend`] from [`TargetWatcher::new`], a test fake
    /// through [`TargetWatcher::with_backend`].
    backend: B,
    /// The live watch map (module docs) — shared with the event
    /// closure, which filters by it.
    dirs: WatchedDirs,
    /// The converged file set: every wanted file whose directory is
    /// actually watched. This, not the last `set_files` input, is the
    /// truth [`TargetWatcher::watched_files`] answers.
    files: BTreeSet<PathBuf>,
    /// The last `set_files` input verbatim — the "wanted" half of
    /// [`TargetWatcher::unwatched_files`]'s difference.
    wanted: BTreeSet<PathBuf>,
}

impl TargetWatcher<NotifyBackend> {
    /// Builds the watcher: one `notify-debouncer-full` debouncer with
    /// the given `debounce` window (SYS-007: 300 ms in production;
    /// tests pass their own) delivering [`WatchEvent`]s to `events`.
    ///
    /// The watcher starts with an empty set — nothing is watched
    /// until the first [`TargetWatcher::set_files`].
    ///
    /// # Errors
    ///
    /// [`WatcherInitError`] when the debouncer or its backend cannot
    /// be constructed (nothing is running then; there is nothing to
    /// degrade to — the caller refuses to start watching).
    pub fn new(debounce: Duration, events: Sender<WatchEvent>) -> Result<Self, WatcherInitError> {
        let dirs: WatchedDirs = Arc::new(Mutex::new(HashMap::new()));
        let backend = NotifyBackend::new(debounce, Arc::clone(&dirs), events)?;
        Ok(Self {
            backend,
            dirs,
            files: BTreeSet::new(),
            wanted: BTreeSet::new(),
        })
    }
}

impl<B: WatchBackend> TargetWatcher<B> {
    /// The seam constructor (module docs, D-137's P3-7): a watcher
    /// over a caller-supplied [`WatchBackend`] instead of the
    /// production debouncer — the entry the watch-limit tests use
    /// to refuse a watch on an existing healthy directory, the one
    /// refusal shape no filesystem fixture can produce. Production
    /// code has no reason past [`TargetWatcher::new`].
    ///
    /// The watcher starts with an empty set, exactly as `new` does.
    pub fn with_backend(backend: B) -> Self {
        Self {
            backend,
            dirs: Arc::new(Mutex::new(HashMap::new())),
            files: BTreeSet::new(),
            wanted: BTreeSet::new(),
        }
    }

    /// Converges the watched set to exactly `files` — the idempotent
    /// full-set update (module docs): directories enter the watch as
    /// needed, leave it when none of their files remains, and are
    /// RE-ARMED otherwise (the backend can lose a watch behind the
    /// caller's back — a deleted-and-recreated directory ends its
    /// inotify watch with a silent `IN_IGNORED` — so "already in the
    /// map" never substitutes for the syscall; `watch` is idempotent
    /// for a live watch). Files under a directory that cannot be
    /// watched are left OUT of the converged set, silently by design:
    /// "no watch" is the honest answer for an unavailable root, not
    /// an error (SYS-007's removal rule; the retry is the next call) —
    /// and the gap stays OBSERVABLE through
    /// [`TargetWatcher::unwatched_files`], the seam SYS-010's notice
    /// and log lines will read. A directory already in the map
    /// STAYS converged even when its re-arm is refused — the map
    /// keeps exactly the evidence it has (the shape where this
    /// bites: the backend LOST the watch behind the caller's back
    /// — a deleted-and-recreated directory's `IN_IGNORED` — and the
    /// limit refuses the re-add; a genuinely LIVE watch's re-arm
    /// cannot be refused by the watches limit at all, the kernel
    /// updates the existing mark without allocating). The opposite
    /// gap — a wanted directory deleted FOR GOOD keeps a phantom
    /// map entry this way — is a recorded follow-up (D-139).
    pub fn set_files(&mut self, files: &[PathBuf]) {
        self.wanted = files.iter().cloned().collect();
        let wanted = wanted_dirs(files);
        // Plan against a snapshot of the live map, run the watch
        // syscalls with NO lock held (the event thread reads the map
        // while they run), then commit the converged result in one
        // short critical section.
        let current: BTreeMap<PathBuf, BTreeSet<OsString>> = {
            let dirs = self.dirs.lock().expect(LOCK_BROKEN);
            dirs.iter()
                .map(|(dir, names)| (dir.clone(), names.clone()))
                .collect()
        };
        let mut converged: HashMap<PathBuf, BTreeSet<OsString>> = HashMap::new();
        for (dir, names) in &current {
            if wanted.contains_key(dir) {
                converged.insert(dir.clone(), names.clone());
            } else {
                // Nothing of this directory is wanted anymore: the
                // watch goes. A failure means the backend already
                // dropped it (or the volume died) — either way the map
                // forgets it, and the next set_files re-adds it
                // cleanly.
                self.backend.unwatch(dir);
            }
        }
        for (dir, names) in &wanted {
            // Unconditional: the backend may have LOST a watch it still
            // reported as live (a deleted-and-recreated directory ends
            // its inotify watch with a silent IN_IGNORED), so every
            // wanted directory is re-armed every time — idempotent for
            // a live watch, healing for a lost one, and the refused
            // one's retry too (the next pass arms it again).
            if self.backend.watch(dir).is_ok() {
                converged.insert(dir.clone(), names.clone());
            }
            // A watch that failed stays absent: the converged set
            // answers what IS watched, never what was asked for.
        }
        {
            let mut dirs = self.dirs.lock().expect(LOCK_BROKEN);
            *dirs = converged;
        }
        self.files = {
            let dirs = self.dirs.lock().expect(LOCK_BROKEN);
            files
                .iter()
                .filter(|file| is_watched_name(&dirs, file))
                .cloned()
                .collect()
        };
    }

    /// The converged set: exactly the files of the last
    /// [`TargetWatcher::set_files`] whose parent directory is
    /// actually watched right now. A BTreeSet for a stable,
    /// order-independent answer.
    #[must_use]
    pub fn watched_files(&self) -> BTreeSet<PathBuf> {
        self.files.clone()
    }

    /// The wanted-but-unwatched set: the files of the last
    /// [`TargetWatcher::set_files`] that have NO watch right now —
    /// the SYS-010 seam. A non-empty answer means the backend refused
    /// a root (an unwatchable folder, the inotify watch limit) and
    /// those files' external edits are invisible until it changes;
    /// the caller's documented responses — the warning line and the
    /// non-blocking notice — read exactly this set. Deliberately NOT
    /// a [`WatchEvent::Backend`] emission: a refused watch is a
    /// persistent fact the next `set_files` re-evaluates, and turning
    /// it into an event would loop (`Backend` → full re-check →
    /// `set_files` → `Backend`) with nothing new to say.
    #[must_use]
    pub fn unwatched_files(&self) -> BTreeSet<PathBuf> {
        self.wanted
            .iter()
            .filter(|file| !self.files.contains(*file))
            .cloned()
            .collect()
    }
}

/// The debouncer callback's body, extracted so both arms are
/// drivable without notify's own event thread (D-137's P3-7: the
/// fault-injection feature cannot reach notify's stream, but the
/// mapping is the contract and it runs here unchanged). An `Ok`
/// batch delivers the name-filtered [`classify_event`] results; a
/// batch that carries the backend's LOST-EVENTS signal delivers ONE
/// extra [`WatchEvent::Backend`] for the whole batch (the
/// sanctioned response is one full re-check, not one per signal);
/// an `Err` batch — the event stream itself failed — is that ONE
/// `Backend` event alone. A closed channel drops every delivery
/// (the contract is delivery while the consumer lives).
///
/// The lost-events signal comes in two shapes (both SYS-010's, both
/// mapped here to the same one Backend event): the event-queue
/// OVERFLOW, which inotify delivers as a rescan flag riding an
/// ordinary `Ok` batch (`IN_Q_OVERFLOW` becomes
/// `EventKind::Other` + `Flag::Rescan` with no paths of its own —
/// `classify_event` correctly maps it to nothing, so the batch-
/// level signal is detected here, before classification), and an
/// `Err` batch (a stream read failure).
fn dispatch(result: DebounceEventResult, dirs: &WatchedDirs, sink: &Sender<WatchEvent>) {
    match result {
        Ok(debounced) => {
            // The map is read for the whole batch under one short
            // lock: set_files never holds it across its watch
            // syscalls (it commits after them), so this arm is
            // never blocked behind a filesystem operation.
            let dirs = dirs.lock().expect(LOCK_BROKEN);
            // SYS-010's overflow rides this batch as the rescan
            // flag; ordinary deliveries go out as usual, and one
            // Backend for the whole batch follows them.
            let mut rescan = false;
            for event in debounced {
                if event.event.need_rescan() {
                    rescan = true;
                    continue;
                }
                for watch_event in classify_event(&event.event.kind, &event.event.paths)
                    .into_iter()
                    .filter(|watch_event| match watch_event {
                        WatchEvent::Changed(path) | WatchEvent::Removed(path) => {
                            is_watched_name(&dirs, path)
                        }
                        // classify_event never emits it.
                        WatchEvent::Backend => false,
                    })
                {
                    // A closed channel (the consumer was dropped)
                    // drops the event: the contract is delivery
                    // while the consumer lives, not outliving it.
                    let _ = sink.send(watch_event);
                }
            }
            drop(dirs);
            if rescan {
                // Same degrade as the Err arm below, one event for
                // the whole batch; the diagnostic line is the app
                // layer's (module docs).
                let _ = sink.send(WatchEvent::Backend);
            }
        }
        Err(_errors) => {
            // SYS-010's degrade: no crash, no silent state — one
            // Backend event tells the consumer to re-check
            // everything once. The diagnostic line is the app
            // layer's (module docs).
            let _ = sink.send(WatchEvent::Backend);
        }
    }
}

/// Whether `path` is one of the names filtered in for its parent
/// directory in the live map. A path with no parent (a root) or a
/// relative path (an empty parent) never matches: the caller's files
/// are absolute repository paths.
fn is_watched_name(dirs: &HashMap<PathBuf, BTreeSet<OsString>>, path: &Path) -> bool {
    path.parent().is_some_and(|parent| {
        !parent.as_os_str().is_empty()
            && dirs
                .get(parent)
                .is_some_and(|names| path.file_name().is_some_and(|name| names.contains(name)))
    })
}

/// Maps one notify event onto the advisory deliveries for its paths:
/// removals and rename-aways are [`WatchEvent::Removed`], creations,
/// modifications and rename-intos are [`WatchEvent::Changed`],
/// access-only noise (reads) is dropped. A rename the backend paired
/// (`RenameMode::Both`) carries its paths as `[source, destination]`
/// — notify's convention — so the source's name is reported gone and
/// the destination's reported changed. The classification is
/// advisory: the consumer's honest state comes from re-reading the
/// file either way (module docs).
fn classify_event(kind: &EventKind, paths: &[PathBuf]) -> Vec<WatchEvent> {
    match kind {
        EventKind::Access(_) => Vec::new(),
        EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => paths
            .iter()
            .map(|path| WatchEvent::Removed(path.clone()))
            .collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                if index == 0 {
                    WatchEvent::Removed(path.clone())
                } else {
                    WatchEvent::Changed(path.clone())
                }
            })
            .collect(),
        _ => paths
            .iter()
            .map(|path| WatchEvent::Changed(path.clone()))
            .collect(),
    }
}

/// Groups the wanted files by parent directory — the desired watch
/// map of [`TargetWatcher::set_files`]. Files with no file name or no
/// usable parent are ignored (malformed input, not a watchable
/// thing).
fn wanted_dirs(files: &[PathBuf]) -> BTreeMap<PathBuf, BTreeSet<OsString>> {
    let mut wanted: BTreeMap<PathBuf, BTreeSet<OsString>> = BTreeMap::new();
    for file in files {
        if let (Some(dir), Some(name)) = (file.parent(), file.file_name()) {
            wanted
                .entry(dir.to_owned())
                .or_default()
                .insert(name.to_owned());
        }
    }
    wanted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::mpsc;
    use std::time::Instant;

    use notify_debouncer_full::DebouncedEvent;
    use notify_debouncer_full::notify::event::{Event, EventKind, Flag};

    /// One event-stream error in notify's own shape. Which variant
    /// it is never reaches the mapping under test: the whole `Err`
    /// batch is the one signal (a stream I/O failure) — so the test
    /// uses the plainest constructor, not an OS errno (D-111: a raw
    /// errno in a test is a platform statement, and nothing here
    /// branches on one).
    fn stream_error() -> notify_debouncer_full::notify::Error {
        notify_debouncer_full::notify::Error::generic("the event stream failed")
    }

    /// The event-queue overflow's exact debounced shape (verified in
    /// the vendored sources, notify 8.2.2 `src/inotify.rs:212-214`
    /// and notify-debouncer-full 0.7 `src/lib.rs:282-284`): inotify
    /// reports `IN_Q_OVERFLOW` as `EventKind::Other` with the
    /// rescan flag and NO paths of its own, and the debouncer
    /// expires it inside an ordinary `Ok` batch — so the real
    /// overflow is publicly constructible and pinned here, no
    /// forcing of notify's stream needed.
    fn rescan_event() -> DebouncedEvent {
        DebouncedEvent::new(
            Event::new(EventKind::Other).set_flag(Flag::Rescan),
            Instant::now(),
        )
    }

    #[test]
    fn a_stream_error_batch_is_delivered_as_exactly_one_backend_event() {
        // A stream error batch (the backend's event stream failed a
        // read): the consumer's signal to re-check everything once
        // arrives — as one Backend event for the whole batch, never
        // a storm of one-per-error.
        let (sender, receiver) = mpsc::channel();
        let dirs: WatchedDirs = Arc::new(Mutex::new(HashMap::new()));
        dispatch(Err(vec![stream_error()]), &dirs, &sender);
        assert_eq!(receiver.try_recv(), Ok(WatchEvent::Backend));
        assert_eq!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty),
            "one Err batch is one Backend event, not one per error"
        );
    }

    #[test]
    fn a_batch_of_two_stream_errors_is_still_one_backend_event() {
        // The count is per BATCH: notify may coalesce several stream
        // failures into one callback, and each of them describes the
        // same fact (the stream is broken) — the one full re-check
        // the Backend event asks for covers them all.
        let (sender, receiver) = mpsc::channel();
        let dirs: WatchedDirs = Arc::new(Mutex::new(HashMap::new()));
        dispatch(Err(vec![stream_error(), stream_error()]), &dirs, &sender);
        assert_eq!(receiver.try_recv(), Ok(WatchEvent::Backend));
        assert_eq!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty),
            "two errors in one batch still ask for one re-check"
        );
    }

    #[test]
    fn an_event_queue_overflow_batch_is_delivered_as_one_backend_event() {
        // SYS-010's event-queue overflow, the real shape: inotify's
        // Q_OVERFLOW rides an ORDINARY batch as the rescan flag
        // (module docs) — classify_event rightly maps the pathless
        // event to nothing, and the batch-level Backend signal is
        // the watcher's whole honest answer. Two rescan notices in
        // one batch are still ONE re-check request.
        let (sender, receiver) = mpsc::channel();
        let dirs: WatchedDirs = Arc::new(Mutex::new(HashMap::new()));
        dispatch(Ok(vec![rescan_event(), rescan_event()]), &dirs, &sender);
        assert_eq!(receiver.try_recv(), Ok(WatchEvent::Backend));
        assert_eq!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty),
            "an overflow batch asks for exactly one re-check"
        );
    }

    #[test]
    fn an_overflow_batch_still_delivers_its_ordinary_events() {
        // The overflow never displaces the batch's own deliveries
        // (the debouncer expires the rescan notice TOGETHER with the
        // window's ordinary events): a watched file's Changed event
        // is delivered, then the one Backend for the batch — the
        // consumer's full re-check covers the file either way, so
        // the order is deliver-then-signal.
        let (sender, receiver) = mpsc::channel();
        let target = PathBuf::from("/repo/AGENTS.md");
        let mut names: HashMap<PathBuf, BTreeSet<OsString>> = HashMap::new();
        names.insert(
            PathBuf::from("/repo"),
            BTreeSet::from([OsString::from("AGENTS.md")]),
        );
        let dirs: WatchedDirs = Arc::new(Mutex::new(names));
        let ordinary = DebouncedEvent::new(
            Event::new(EventKind::Any).add_path(target.clone()),
            Instant::now(),
        );
        dispatch(Ok(vec![rescan_event(), ordinary]), &dirs, &sender);
        assert_eq!(receiver.try_recv(), Ok(WatchEvent::Changed(target.clone())));
        assert_eq!(receiver.try_recv(), Ok(WatchEvent::Backend));
        assert_eq!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty),
            "the mixed batch delivers its Changed event and one Backend, nothing else"
        );
    }

    #[test]
    fn every_send_site_drops_delivery_to_a_dead_consumer_instead_of_panicking() {
        // The delivery contract is "while the consumer lives": a
        // dropped receiver must not panic the event thread — a
        // panicking debouncer thread dies silently (no events, no
        // error), which is exactly the crash SYS-010 forbids in
        // miniature. One dead consumer, then a batch through EVERY
        // send site: an ordinary Changed delivery, the overflow's
        // trailing Backend, and a stream-error batch's Backend —
        // each site's `let _` is its own code, so each must be
        // pinned. The falsifiable form: re-introduce an expect() on
        // any of the three sends and this test panics; reaching the
        // end is the surviving behavior the module docs promise.
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        let target = PathBuf::from("/repo/AGENTS.md");
        let mut names: HashMap<PathBuf, BTreeSet<OsString>> = HashMap::new();
        names.insert(
            PathBuf::from("/repo"),
            BTreeSet::from([OsString::from("AGENTS.md")]),
        );
        let dirs: WatchedDirs = Arc::new(Mutex::new(names));
        let ordinary =
            DebouncedEvent::new(Event::new(EventKind::Any).add_path(target), Instant::now());
        // The ordinary delivery and the overflow's Backend:
        dispatch(Ok(vec![rescan_event(), ordinary]), &dirs, &sender);
        // The stream-error batch's Backend:
        dispatch(Err(vec![stream_error()]), &dirs, &sender);
    }
}
