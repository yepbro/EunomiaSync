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
//! previous set was. A directory that CANNOT be watched — the root
//! is gone, a dead drive, permissions — is not an error: the file
//! simply leaves the converged set [`TargetWatcher::watched_files`]
//! answers, and the next `set_files` retries it (an unavailable
//! project has no watch, per SYS-007's removal rule; the
//! availability gate itself is the caller's) — with one deliberate
//! exception: a refusal that is no evidence about the watch's life
//! (class [`WatchRefusalKind::Other`]) leaves a converged entry's
//! carried liveness standing. Nothing panics and nothing is
//! watched half-way: the map is the truth — and the map has TWO
//! halves, the names (the event filter) and each entry's LIVENESS
//! (the reporting truth). The re-arm every pass performs is the
//! liveness probe: an armed watch is live, a refusal of class
//! [`WatchRefusalKind::Gone`] or [`Limit`] proves the watch cannot
//! exist (the phantom shapes D-139 recorded — a directory deleted
//! for good, a watch lost to a delete-recreate with the limit
//! blocking the re-add — no longer count as watched and surface in
//! [`TargetWatcher::unwatched_files`] instead), and a refusal of
//! class [`WatchRefusalKind::Other`] is no evidence, the entry's
//! last known liveness standing.
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
//! the file stays out of the converged set (freshly refused, or —
//! once the refusal's class proves the watch dead, the liveness
//! rule above — refused on re-arm), and the caller
//! reads the gap from [`TargetWatcher::unwatched_files`] instead
//! (turning a refusal into an event would loop: `Backend` →
//! re-check → `set_files` → `Backend`, with nothing new to say).
//! [`TargetWatcher::unwatched_kinds`] answers the same gap WITH the
//! refusal's evidence class per file — the form the SYS-010 warning
//! line and non-blocking notice consume (D-155). The classification
//! is advisory — honest state comes from
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
//! (the unwatched seam answers exactly them) while the directories
//! armed before the hit keep their convergence on that first pass;
//! the NEXT pass's refused re-arm of such a directory is read as
//! proof the watch was lost (the liveness rule above), so from
//! there on the whole set answers honestly unwatched, identically
//! on every repeated pass, until lifting the
//! limit lets the very next `set_files` converge everything; and
//! every pass re-arms every wanted directory (the refused ones
//! included — the idempotent retry). The liveness battery (D-139's
//! phantom-entry follow-up): a wanted directory deleted for good
//! stops counting as watched and reports the gap until it returns
//! (a real backend), a limit-refused re-arm of a converged
//! directory does the same (the refusal proves the watch lost),
//! while a refusal of class `Other` keeps a converged watch
//! counted (no evidence, no overreach). The event-queue overflow
//! half is
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

impl WatcherInitError {
    /// Wraps the class-only diagnostic — the backend's own error text,
    /// never file content. The construction seam for callers that need
    /// to state an init failure the backend did not produce (the
    /// logging facade's line tests; a fake backend's refusal),
    /// mirroring [`WatchRefused::new`].
    pub fn new(reason: String) -> Self {
        Self(reason)
    }
}

impl fmt::Display for WatcherInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the file watcher backend failed to start: {}", self.0)
    }
}

impl std::error::Error for WatcherInitError {}

/// The class of a [`WatchRefused`]: what the refusal PROVES about the
/// watch. This is the evidence [`TargetWatcher::set_files`] reads to
/// keep its liveness honest (D-139's recorded follow-up: a converged
/// entry whose re-arm is refused must stop counting as watched when —
/// and only when — the refusal proves the watch cannot exist).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchRefusalKind {
    /// The directory watch cannot exist because the path is not a
    /// directory anymore: the production backend establishes this by
    /// the STAT (a missing path, or a file standing where the
    /// directory was) before it arms anything, because notify's
    /// backends disagree on how a missing path reaches the caller
    /// (the Windows backend refuses with a generic error — see
    /// [`NotifyBackend::watch`]); inotify's `PathNotFound` maps here
    /// too. Any watch the directory had cannot exist: the kernel
    /// ends a deleted directory's watch with the silent `IN_IGNORED`.
    Gone,
    /// The watch-limit refusal (`fs.inotify.max_user_watches`,
    /// SYS-010; notify's `MaxFilesWatch`). The kernel never refuses
    /// the re-arm of a genuinely live watch — it updates the existing
    /// mark without allocating — so a limit refusal of a converged
    /// directory proves the watch was LOST.
    Limit,
    /// Any other refusal (an I/O error, an unreachable mount,
    /// permissions): NOT evidence about the watch's life — an
    /// existing kernel watch keeps delivering through one — so the
    /// entry's last known liveness stands.
    Other,
}

/// The refusal of one directory watch: the backend would not arm a
/// watch on the directory — on Linux the WATCHES limit
/// (`fs.inotify.max_user_watches`, surfaced by notify 8.2 as
/// `MaxFilesWatch` after the kernel's `ENOSPC`), elsewhere an
/// unwatchable or vanished root. (The separate INSTANCES limit
/// `EMFILE` never gets this far: `inotify_init` fails at watcher
/// CONSTRUCTION, which is [`WatcherInitError`]'s no-watch-at-all
/// degrade — adding a watch allocates no descriptor.) Carries the
/// refusal's [`WatchRefusalKind`] — the evidence class the
/// convergence reads — and a class-only diagnostic string, never
/// file content. The watcher's response never branches on the
/// reason text: a refusal is a persistent fact about the directory,
/// answered by the converged set and
/// [`TargetWatcher::unwatched_files`], not an error and not an
/// event (module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRefused {
    /// What the refusal proves about the watch (see
    /// [`WatchRefusalKind`]).
    kind: WatchRefusalKind,
    /// The class-only diagnostic (an OS error's kind or code name),
    /// never file content.
    reason: String,
}

impl WatchRefused {
    /// Wraps the refusal's class and the backend's reason — its class
    /// text (an OS error's kind or code name), never file content.
    pub fn new(kind: WatchRefusalKind, reason: String) -> Self {
        Self { kind, reason }
    }

    /// The refusal's evidence class: what this refusal proves about
    /// the watch (see [`WatchRefusalKind`]).
    #[must_use]
    pub fn kind(&self) -> WatchRefusalKind {
        self.kind
    }
}

impl fmt::Display for WatchRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the file watcher backend refused the directory: {}",
            self.reason
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
    /// refusal's [`WatchRefusalKind`] is the evidence the caller
    /// reads for its liveness bookkeeping; the caller's response is
    /// the converged set, not the error.
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
        // The gone-for-good fact is established by the STAT, not by
        // the backend's refusal vocabulary: notify's backends
        // disagree on how a missing path reaches the caller (inotify
        // remaps the kernel's ENOENT to `PathNotFound` — 8.2.2
        // `src/inotify.rs:451-457` — and FSEvents pre-checks with
        // `path_not_found()` — `src/fsevent.rs:372-375` — but the
        // Windows backend refuses with a GENERIC error before any
        // syscall — `src/windows.rs:557-564`), so the error kind
        // alone cannot carry the class. The stat is one metadata
        // probe per directory per full pass, beside the watch
        // syscall the pass already pays; any other stat failure is
        // no evidence and falls through to the backend, whose own
        // refusal (if any) classifies there — as does the TOCTOU
        // race (the directory vanishing between the stat and the
        // syscall).
        match std::fs::metadata(dir) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_not_a_directory) => {
                return Err(WatchRefused::new(
                    WatchRefusalKind::Gone,
                    "the path is not a directory".to_owned(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(WatchRefused::new(
                    WatchRefusalKind::Gone,
                    "the directory does not exist".to_owned(),
                ));
            }
            // An unproven stat (permissions, a dying mount) says
            // nothing: let the backend's arming decide.
            Err(_unproven) => {}
        }
        self.debouncer
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|error| WatchRefused::new(refusal_kind(&error), error.to_string()))
    }

    fn unwatch(&mut self, dir: &Path) {
        let _ = self.debouncer.unwatch(dir);
    }
}

/// Maps notify's platform-neutral error kind onto the refusal's
/// evidence class ([`WatchRefusalKind`]) — the FALLBACK half of the
/// production classification, reached when the stat pre-check in
/// [`NotifyBackend::watch`] proved nothing and the backend's own
/// arming refused: verified in the registry sources, notify 8.2.2
/// `src/inotify.rs:451-457` — the watches limit's kernel `ENOSPC`
/// is remapped to `MaxFilesWatch`, an io `NotFound` (the TOCTOU
/// race) to `PathNotFound` — and the debouncer hands the watcher's
/// synchronous `Result` through untouched (notify-debouncer-full
/// 0.7.0 `src/lib.rs:600-608`). Everything else stays `Other`: an
/// I/O or permissions refusal is not evidence about a watch's life.
fn refusal_kind(error: &notify_debouncer_full::notify::Error) -> WatchRefusalKind {
    use notify_debouncer_full::notify::ErrorKind;
    match &error.kind {
        ErrorKind::PathNotFound => WatchRefusalKind::Gone,
        ErrorKind::MaxFilesWatch => WatchRefusalKind::Limit,
        _ => WatchRefusalKind::Other,
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
    /// The liveness half of the converged map: the wanted directories
    /// whose watch the backend ARMED at the last `set_files` pass, or
    /// whose last known liveness stands because a later re-arm
    /// refusal carried no evidence (class [`WatchRefusalKind::Other`]
    /// — D-139's phantom-entry follow-up). Kept beside the shared
    /// `dirs` map, not inside it: the debouncer's event thread
    /// filters by NAMES only (a dead watch delivers nothing anyway,
    /// and a carried-live entry must keep delivering), so liveness
    /// never crosses that lock — only `set_files` and the queries
    /// touch it, all on the caller's side. Invariant: every live
    /// directory is a key of the map.
    live: BTreeSet<PathBuf>,
    /// The refusal-evidence half: the CLASS of the last refusal each
    /// wanted directory's re-arm answered, cleared by a successful
    /// arm and dropped when the directory leaves the wanted set. The
    /// same evidence `set_files` reads for its liveness bookkeeping
    /// (the `kind` the backend refused with), kept so
    /// [`TargetWatcher::unwatched_kinds`] can explain every
    /// wanted-but-unwatched file with its parent directory's class —
    /// the SYS-010 notice and warning line's data (D-155). A file
    /// with no recorded parent class answers [`WatchRefusalKind::
    /// Other`] — the honest catch-all, not evidence of anything.
    refused: HashMap<PathBuf, WatchRefusalKind>,
    /// The converged file set: every wanted file whose directory is
    /// actually watched LIVE right now. This, not the last
    /// `set_files` input and not bare map membership, is the truth
    /// [`TargetWatcher::watched_files`] answers.
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
            live: BTreeSet::new(),
            refused: HashMap::new(),
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
            live: BTreeSet::new(),
            refused: HashMap::new(),
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
    /// [`TargetWatcher::unwatched_kinds`] (the class-carrying form
    /// the SYS-010 notice and warn line read, D-155) and its path-set
    /// twin [`TargetWatcher::unwatched_files`] (the form this crate's
    /// own battery asserts on).
    ///
    /// The re-arm's outcome is EVIDENCE about the watch's life, read
    /// per the refusal's [`WatchRefusalKind`] (the liveness half of
    /// the map, D-139's phantom-entry follow-up): an armed watch is
    /// live; a refused re-arm of class `Gone` or `Limit` PROVES the
    /// watch cannot exist (the directory is gone, or — the kernel
    /// never refuses a genuinely live watch's re-arm — the watch was
    /// lost and the limit blocks the re-add), so the entry stops
    /// counting as watched even though the map keeps its names (a
    /// converged entry is never evicted: the names stay the event
    /// filter's evidence); a refusal of class `Other` is no evidence
    /// either way, and the entry's last known liveness stands. The
    /// unconditional re-arm is therefore also the whole recovery: a
    /// directory that comes back arms `Ok` on the very next pass,
    /// with no flag to clear.
    pub fn set_files(&mut self, files: &[PathBuf]) {
        self.wanted = files.iter().cloned().collect();
        let wanted = wanted_dirs(files);
        // The refusal evidence of directories no longer wanted goes
        // with them: the leaving-set loop below iterates the CONVERGED
        // map, which never held a never-armed refused directory, so
        // the prune cannot live there (the round-1 P3-1 fix) — a
        // stale entry could never change an answer (every wanted
        // directory is re-armed and its record overwritten before any
        // read), but the map would grow by one entry per distinct
        // refused-then-removed directory for the session.
        self.refused.retain(|dir, _| wanted.contains_key(dir));
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
                // watch goes, and with it the directory's liveness. A
                // failure means the backend already dropped it (or the
                // volume died) — either way the map forgets it, and
                // the next set_files re-adds it cleanly.
                self.backend.unwatch(dir);
                self.live.remove(dir);
            }
        }
        for (dir, names) in &wanted {
            // Unconditional: the backend may have LOST a watch it still
            // reported as live (a deleted-and-recreated directory ends
            // its inotify watch with a silent IN_IGNORED), so every
            // wanted directory is re-armed every time — idempotent for
            // a live watch, healing for a lost one, and the refused
            // one's retry too (the next pass arms it again).
            match self.backend.watch(dir) {
                Ok(()) => {
                    converged.insert(dir.clone(), names.clone());
                    self.live.insert(dir.clone());
                    // An armed watch needs no refusal explanation;
                    // a previous pass's refusal stops describing it.
                    self.refused.remove(dir);
                }
                Err(refused) => {
                    // The refusal's class is the evidence: `Gone` and
                    // `Limit` prove the watch cannot exist (a phantom
                    // entry would lie — watched_files must not count
                    // it), `Other` says nothing about the watch and
                    // the carried liveness stands. Either way the
                    // class is the last fact known about the
                    // directory — the evidence unwatched_kinds
                    // explains the gap with (D-155).
                    self.refused.insert(dir.clone(), refused.kind());
                    if refused.kind() != WatchRefusalKind::Other {
                        self.live.remove(dir);
                    }
                }
            }
        }
        {
            let mut dirs = self.dirs.lock().expect(LOCK_BROKEN);
            *dirs = converged;
        }
        self.files = {
            let dirs = self.dirs.lock().expect(LOCK_BROKEN);
            files
                .iter()
                .filter(|file| is_live_watched_name(&dirs, &self.live, file))
                .cloned()
                .collect()
        };
    }

    /// The converged set: exactly the files of the last
    /// [`TargetWatcher::set_files`] whose parent directory's watch is
    /// LIVE right now — armed at the last pass, or carried through a
    /// non-evidential refusal (the liveness half of the map; a
    /// refused re-arm that proves the watch dead leaves the names in
    /// the filter map but does not count). A BTreeSet for a stable,
    /// order-independent answer.
    #[must_use]
    pub fn watched_files(&self) -> BTreeSet<PathBuf> {
        self.files.clone()
    }

    /// The wanted-but-unwatched set: the files of the last
    /// [`TargetWatcher::set_files`] that have NO live watch right
    /// now — the path-set half of the SYS-010 seam (this crate's own
    /// battery asserts on it; the app layer reads the class-carrying
    /// [`TargetWatcher::unwatched_kinds`]). A non-empty answer means
    /// the backend refused a root (an unwatchable folder, the inotify
    /// watch limit) or proved a converged watch dead by refusing its
    /// re-arm (the directory gone for good, the watch lost to a
    /// delete-recreate with the limit blocking the re-add) — the
    /// phantom shapes D-139 recorded, honestly reported here since
    /// the liveness fix — and those files' external edits are
    /// invisible until the backend arms them again (the next
    /// `set_files` retries unconditionally). Deliberately NOT a
    /// [`WatchEvent::Backend`] emission: a refused watch is a
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

    /// The wanted-but-unwatched files WITH the refusal class that
    /// explains each — the class-carrying form of
    /// [`TargetWatcher::unwatched_files`], added when the SYS-010
    /// notice and warning line became its consumers (D-155, the
    /// change D-139's round-1 P3-1 and D-148 both deferred until a
    /// consumer existed): every unwatched file answers the class its
    /// parent directory's last re-arm refusal carried — `Gone` (the
    /// directory is not there), `Limit` (the watch budget refused the
    /// arm), `Other` (some non-evidential refusal, or a file whose
    /// parent holds no recorded class — the honest catch-all, never a
    /// guess). The classes are the PASS's own evidence: a directory
    /// armed `Ok` clears its record, so the answer always describes
    /// the last `set_files`, never stale history. Same refusal-set
    /// discipline as [`TargetWatcher::unwatched_files`]: a persistent
    /// polled fact, not an event.
    #[must_use]
    pub fn unwatched_kinds(&self) -> BTreeMap<PathBuf, WatchRefusalKind> {
        self.wanted
            .iter()
            .filter(|file| !self.files.contains(*file))
            .map(|file| {
                let kind = file
                    .parent()
                    .and_then(|parent| self.refused.get(parent).copied())
                    .unwrap_or(WatchRefusalKind::Other);
                (file.clone(), kind)
            })
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

/// Whether `path` counts as converged-watched: its parent directory
/// holds a map entry naming it AND that entry's watch is LIVE — the
/// liveness half of the map decides the reports (a refused re-arm
/// that proves the watch dead leaves the names in the filter map,
/// where they are inert — a dead watch delivers nothing — but must
/// not count as watched).
fn is_live_watched_name(
    dirs: &HashMap<PathBuf, BTreeSet<OsString>>,
    live: &BTreeSet<PathBuf>,
    path: &Path,
) -> bool {
    is_watched_name(dirs, path) && path.parent().is_some_and(|parent| live.contains(parent))
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
    use std::time::{Duration, Instant};

    use notify_debouncer_full::DebouncedEvent;
    use notify_debouncer_full::notify::event::{Event, EventKind, Flag};
    use notify_debouncer_full::notify::{Error as NotifyError, ErrorKind as NotifyErrorKind};

    #[test]
    fn notify_error_kinds_map_onto_the_refusal_evidence_classes() {
        // The FALLBACK half of the production classification, pinned
        // by value against notify's own kind vocabulary (verified in
        // the registry sources, notify 8.2.2 `src/inotify.rs:451-
        // 457`: the watches limit's kernel ENOSPC arrives remapped as
        // `MaxFilesWatch`, an io `NotFound` — the TOCTOU race — as
        // `PathNotFound`). Everything else is `Other`: not evidence
        // about a watch's life.
        assert_eq!(
            refusal_kind(&NotifyError::new(NotifyErrorKind::PathNotFound)),
            WatchRefusalKind::Gone
        );
        assert_eq!(
            refusal_kind(&NotifyError::new(NotifyErrorKind::MaxFilesWatch)),
            WatchRefusalKind::Limit
        );
        assert_eq!(
            refusal_kind(&NotifyError::generic("an I/O failure")),
            WatchRefusalKind::Other
        );
        assert_eq!(
            refusal_kind(&NotifyError::io(std::io::Error::other("a dying drive"))),
            WatchRefusalKind::Other
        );
    }

    #[test]
    fn the_production_backend_refuses_a_gone_directory_as_gone_on_every_platform() {
        // The ROUND-1 P1 pin: notify's backends disagree on how a
        // missing path reaches the caller — inotify remaps ENOENT to
        // `PathNotFound`, but the Windows backend refuses with a
        // GENERIC error before any syscall (notify 8.2.2
        // `src/windows.rs:557-564`) — so the kind mapping alone
        // cannot classify it and the production backend's stat
        // pre-check owns the class. Both gone shapes through the
        // REAL constructor: a path that does not exist, and a path
        // that exists but is not a directory (on Linux a bare
        // inotify watch on a FILE would arm — a phantom of another
        // flavor; the pre-check refuses it as Gone everywhere).
        let (sender, _receiver) = mpsc::channel();
        let dirs: WatchedDirs = Arc::new(Mutex::new(HashMap::new()));
        let mut backend = NotifyBackend::new(Duration::from_millis(10), dirs, sender)
            .expect("constructing the debouncer must work");
        let missing = std::env::temp_dir().join("eunomia-sync-watch-gone-test");
        let _ = std::fs::remove_dir_all(&missing); // a previous run's debris, if any
        let refused = backend
            .watch(&missing)
            .expect_err("a missing directory cannot be armed");
        assert_eq!(
            refused.kind(),
            WatchRefusalKind::Gone,
            "the missing directory is Gone whatever the platform backend"
        );
        let not_a_directory = std::env::temp_dir().join("eunomia-sync-watch-file-test.txt");
        std::fs::write(&not_a_directory, "a file, not a directory")
            .expect("writing the fixture must work");
        let refused = backend
            .watch(&not_a_directory)
            .expect_err("a file cannot be armed as a directory watch");
        assert_eq!(
            refused.kind(),
            WatchRefusalKind::Gone,
            "a file standing where the directory was is Gone everywhere"
        );
        let _ = std::fs::remove_file(&not_a_directory);
    }

    #[test]
    fn a_never_armed_refusal_leaves_no_evidence_once_its_directory_is_unwanted() {
        // The evidence map's prune (D-155's round-1 P3-1 fix): the
        // leaving-set loop of `set_files` iterates the CONVERGED map,
        // which never held a never-armed refused directory, so the
        // prune must run against the wanted set up front. White-box on
        // `refused` deliberately: the leak is not observable through
        // the public queries (every wanted directory's record is
        // rewritten by the unconditional re-arm before any read), so
        // this unit module — the one place that can see the field —
        // owns the pin.
        struct AlwaysRefusing;
        impl WatchBackend for AlwaysRefusing {
            fn watch(&mut self, _dir: &Path) -> Result<(), WatchRefused> {
                Err(WatchRefused::new(
                    WatchRefusalKind::Limit,
                    "always refused".to_owned(),
                ))
            }
            fn unwatch(&mut self, _dir: &Path) {}
        }
        let file = std::env::temp_dir()
            .join("eunomia-sync-watch-unwanted")
            .join("AGENTS.md");
        let mut watcher = TargetWatcher::with_backend(AlwaysRefusing);
        watcher.set_files(std::slice::from_ref(&file));
        assert_eq!(watcher.unwatched_kinds().len(), 1);
        assert_eq!(
            watcher.refused.len(),
            1,
            "the never-armed refusal recorded its class"
        );
        watcher.set_files(&[]);
        assert!(
            watcher.refused.is_empty(),
            "a directory that left the wanted set leaves no evidence behind"
        );
    }

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
