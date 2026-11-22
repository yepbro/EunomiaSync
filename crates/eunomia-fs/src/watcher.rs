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
//! (reads) is dropped. An error in the backend's EVENT STREAM (an
//! event-queue overflow — SYS-010's case) arrives as
//! [`WatchEvent::Backend`]: the caller's documented response is to
//! re-check every target file once, and the logging of the failure
//! belongs to the app layer's facade. A REFUSED WATCH (an
//! unwatchable root, the inotify watch limit) is a different,
//! persistent fact: it never reaches the event stream — the file
//! simply stays out of the converged set, and the caller reads the
//! gap from [`TargetWatcher::unwatched_files`] instead (turning a
//! refusal into an event would loop: `Backend` → re-check →
//! `set_files` → `Backend`, with nothing new to say). The
//! classification is advisory — honest state comes from re-reading
//! the file — but it is tested as delivered.
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
    /// The watcher backend's EVENT STREAM reported an error (an
    /// event-queue overflow — SYS-010's case; a refused watch is NOT
    /// this, see [`TargetWatcher::unwatched_files`]). The failure's
    /// details belong to the app layer's logging; the sanctioned
    /// reaction here is one full re-check of every target file.
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
pub struct TargetWatcher {
    /// The debouncer guard; owns the backend thread and stops it on
    /// drop. `watch`/`unwatch` are the only calls made on it.
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
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

impl TargetWatcher {
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
        let filter_dirs = Arc::clone(&dirs);
        let sink = events;
        let debouncer = new_debouncer(debounce, None, move |result: DebounceEventResult| {
            match result {
                Ok(debounced) => {
                    // The map is read for the whole batch under one
                    // short lock: set_files never holds it across its
                    // watch syscalls (it commits after them), so this
                    // closure is never blocked behind a filesystem
                    // operation.
                    let dirs = filter_dirs.lock().expect(LOCK_BROKEN);
                    for event in debounced {
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
                            // A closed channel (the consumer was
                            // dropped) drops the event: the contract
                            // is delivery while the consumer lives,
                            // not outliving it.
                            let _ = sink.send(watch_event);
                        }
                    }
                }
                Err(_errors) => {
                    // SYS-010's degrade: no crash, no silent state —
                    // one Backend event tells the consumer to
                    // re-check everything once. The diagnostic line is
                    // the app layer's (module docs).
                    let _ = sink.send(WatchEvent::Backend);
                }
            }
        })
        .map_err(|error| WatcherInitError(error.to_string()))?;
        Ok(Self {
            debouncer,
            dirs,
            files: BTreeSet::new(),
            wanted: BTreeSet::new(),
        })
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
    /// and log lines will read.
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
                // watch goes. A failure here means the backend already
                // dropped it (or the volume died) — either way the map
                // forgets it, and the next set_files re-adds it
                // cleanly.
                let _ = self.debouncer.unwatch(dir);
            }
        }
        for (dir, names) in &wanted {
            // Unconditional: the backend may have LOST a watch it still
            // reported as live (a deleted-and-recreated directory ends
            // its inotify watch with a silent IN_IGNORED), so every
            // wanted directory is re-armed every time — idempotent for
            // a live watch, healing for a lost one.
            if self
                .debouncer
                .watch(dir, RecursiveMode::NonRecursive)
                .is_ok()
            {
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
