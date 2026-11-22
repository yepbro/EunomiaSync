//! Repository tree scanning (docs/spec.md 5.2, 9.6).
//!
//! [`scan_repositories`] walks a root folder and reports, entry by entry,
//! what the repository scan found: repositories (folders containing a
//! `.git` directory), plain directories, and skips — a folder whose name
//! is excluded from the scan ([`SkipReason::Excluded`]), a folder whose
//! `.git` is not a directory ([`SkipReason::SubmoduleOrWorktree`], the
//! submodule or linked-worktree form), a folder the OS refused to
//! inspect ([`SkipReason::Access`], carrying the OS error code,
//! PRJ-007), and a folder whose PATH is not valid UTF-8
//! ([`SkipReason::NotUtf8`] — the walk's half of the D-086 addendum:
//! such a path can never become a registered identity, so it is a
//! skip, never a keyed find). The consumer drives the walk through
//! one callback and learns from the returned [`ScanOutcome`] whether
//! it ran to completion or was cancelled.
//!
//! # What this module is, and is not
//!
//! This is the folder traversal of ALG-050..054 and nothing else. It is
//! the walker only: the per-found-path `path_key` and classification
//! into `new`/`known`/`archived`/`excluded` (ALG-053) is a later task;
//! the throttled progress events (SYS-003's `scan://progress`, no more
//! than 10 per second) are the command layer's. Depth and the excluded
//! names are FUNCTION PARAMETERS ([`ScanOptions`]) — this module never
//! reads `config.toml`, the settings table, or the database; the command
//! layer builds `ScanOptions` from the §6.5 settings keys
//! (`scan.max_depth`, D-04; `scan.exclude_names`, D-05) and owns their
//! 1–20 / list validation. `walkdir` stays available for plain
//! single-threaded traversal outside the repository-scan algorithm (for
//! example, the app's own data directory).
//!
//! # The walk configuration (ALG-050, D-07)
//!
//! `ignore`'s parallel [`WalkBuilder`] drives the walk, with every
//! ignore-file source turned OFF: `.gitignore`, `.git/info/exclude`,
//! global gitignore, `.ignore` files, and parent-directory rules. A
//! repository living under an ignored directory must stay visible, and
//! no ignore rule may thin the scan behind the caller's back. The
//! hidden-entry filter is off too: the spec's only skip rules are the
//! explicit excluded names and system folders (ALG-051), and a blanket
//! hidden filter would hide a repository inside, say, `~/projects/.old`
//! that the spec wants found. Symlinks are not followed (D-07); Windows
//! junctions surface as symlinks in std's file type, so the same gate
//! covers ALG-050's reparse-point rule. The ROOT is the exception the
//! rule cannot have: the builder resolves the root with links followed
//! (the user chose that path explicitly, and the availability check
//! accepts a symlinked project root the same way), so a root that is a
//! symlink is traversed as the folder it names, while every link BELOW
//! the root is left alone. A root that is not a directory at all
//! produces no event and a completed walk. Threads are
//! `min(4, available cores)` (ALG-050 names the cap), and the depth
//! parameter is passed through to `WalkBuilder::max_depth` one for one:
//! the walk visits the root and `max_depth` levels of folders below it.
//!
//! # How a folder is classified (ALG-052)
//!
//! Every traversed directory gets exactly one probe:
//! `symlink_metadata(<folder>/.git)` — the same `.git`-check idiom the
//! availability check uses (DECISIONS.md D-084). A `.git` DIRECTORY
//! makes the folder a found repository; the walk then continues INSIDE
//! it, so nested repositories are found separately (ALG-051). A `.git`
//! entry of any other kind — a file (the submodule/worktree gitdir
//! pointer), or a symlink to one — skips the folder with
//! [`SkipReason::SubmoduleOrWorktree`]: the folder is not a plain
//! repository, and per D-06 the user can still add it by hand; the walk
//! does not descend into it. No `.git` entry makes it a plain directory
//! that is merely reported and descended.
//!
//! The probe is the one syscall per directory beyond the walk itself.
//! ALG-054's guarantee ("the scan reads only directory entry names and
//! does not open files") holds: no file is ever opened, no content is
//! ever read, and the probe reads metadata only — nothing a file
//! contains can surface through this API. Watching for the `.git` entry
//! in the walk's own listing instead would seem to save that one stat,
//! but it breaks classification: `.git` is on the default exclusion
//! list (D-05), and exclusion is a traversal rule — an excluded name
//! stops the DESCENT (ALG-051), it must not stop DETECTION of the
//! parent repository, or the default settings would find no
//! repositories at all. The probe keeps the two rules independent.
//!
//! # Skips and access errors (ALG-051, ALG-052, PRJ-007)
//!
//! A folder whose name is on the caller's [`ScanOptions::exclude_names`]
//! or is one of the fixed system folders (`$RECYCLE.BIN`, `System Volume
//! Information`, `.Trash`, `.Trashes`) is skipped with
//! [`SkipReason::Excluded`] — the same reason the UI shows for both
//! (PRJ-002) — and is not descended into, not probed, and not classified
//! even if it holds a `.git` directory: exclusion wins over detection,
//! which is exactly what makes D-05's default list (which contains
//! `.git`) coherent. The scan ROOT is exempt: the user asked for that
//! folder explicitly, so its name never filters it.
//!
//! A folder the OS refuses — a `chmod 000` directory, a drive dropped
//! mid-walk — is skipped with [`SkipReason::Access`] carrying the
//! reconstructed [`io::Error`] (`raw_os_error` and `ErrorKind`
//! preserved: §10's first-class log facts), and the traversal continues
//! with the siblings (PRJ-007). The refusal never aborts the walk and
//! never surfaces as a function-level failure: every access problem is
//! data on an event, never a `Result`.
//!
//! # The UTF-8 identity gate (the D-086 addendum)
//!
//! A folder whose path text is not valid UTF-8 — its own name, or any
//! ancestor's, carries bytes no TEXT identity can hold on a
//! byte-preserving filesystem — is skipped with
//! [`SkipReason::NotUtf8`] and NOT descended into: every descendant's
//! path inherits the invalid bytes, so the whole subtree is
//! unregistrable and walking it could only produce more skips. The
//! gate runs on the WHOLE path at every depth, the scan root
//! included: unlike the excluded-name filter (a policy the user chose
//! and the root is exempt from), this is the TEXT boundary of the
//! registered identity itself — a non-UTF-8 root could never key,
//! store, or be added (the add seam refuses it typed, the same
//! addendum). The command layer reaches this arm only for roots it
//! did not canonicalize through the identity seam; the skip exists so
//! the walk stays honest about every tree it can see. The event's
//! path carries the real bytes (`PathBuf`), and the reason's Display
//! names the encoding fact; the lossy spelling the log and dialog
//! would show is the consumer's rendering, never a key.
//!
//! # Cancellation (ALG-053, SYS-003)
//!
//! The [`CancelToken`] is checked before anything else happens for every
//! entry — found repositories, plain directories, skips, and walker
//! errors alike. A flipped token makes the observing walker thread quit
//! the whole walk promptly, and [`scan_repositories`] reports
//! [`ScanOutcome::Cancelled`] — exactly when a thread observed the flip,
//! not when a flip merely raced with the end of the walk (see
//! [`ScanOutcome`]). The events already delivered are whatever was in
//! flight, and whether to trust or drop them is the command layer's
//! rule (PRJ-002: cancelling a scan adds no projects). The token is
//! [`Clone`] so the command layer can hold one in its state and hand a
//! clone to every scan it starts; SYS-003 gives SYNC the same
//! primitive, and the sync task is expected to reuse this type or
//! relocate it — not to grow a second one.
//!
//! # Ordering
//!
//! The walk is parallel, so EVENT ORDER IS NOT SPECIFIED beyond one
//! structural guarantee: a folder's own event always precedes events for
//! its contents (the walker cannot list a folder before visiting it).
//! The interleaving of events from different folders is arbitrary and
//! must not be depended on; consumers accumulate, they do not sequence.
//! Two events for one path are possible in one corner: a folder that is
//! searchable but not listable (Unix `x` without `r`) is probed and
//! reported (directory or repository), and its failed listing then
//! arrives as an [`SkipReason::Access`] skip for the same path — both
//! facts are true, and both are delivered.
//!
//! # Errors
//!
//! [`scan_repositories`] is total: it never panics on odd filesystem
//! state and returns no `Result`. Every per-entry failure is carried on
//! a [`ScanEvent::Skipped`] with its [`SkipReason`]; a root that cannot
//! be inspected at all yields exactly one such skip.
//!
//! # Testing plan
//!
//! Tests go through the public API against real temporary trees
//! (colocated, house style): nested repositories found separately, the
//! `.git`-file skip with its exact reason, excluded names and the four
//! system folders (skipped and not descended), the depth bound, symlink
//! non-following, `.gitignore` blindness, the root as a repository and
//! the root bypassing the name filters, an unreadable folder and an
//! unreadable root (skipped loudly under root, OS code carried), a
//! searchable-but-unlistable folder's two events, an unreadable FILE
//! producing no event at all (nothing reads contents), the non-UTF-8
//! repository skipped with its own reason and its subtree not walked
//! (Linux-only fixture, the D-086 addendum), the Display and
//! `os_code()` contract, and cancellation — pre-flipped (no events) and
//! mid-walk (the flip lands on the first yielded entry; strictly fewer
//! than all events arrive). Parallel delivery means every assertion is
//! on SETS of paths, never on sequences.

use std::fmt;
use std::fs;
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ignore::{WalkBuilder, WalkState};

/// The system folders the scan always skips (ALG-051), spelled exactly
/// as the platforms name them. Unlike [`ScanOptions::exclude_names`]
/// this list is fixed: it is not configurable, and it does not apply to
/// an explicitly chosen scan root.
const SYSTEM_FOLDER_NAMES: [&str; 4] = [
    "$RECYCLE.BIN",
    "System Volume Information",
    ".Trash",
    ".Trashes",
];

/// The thread cap of ALG-050 ("no more than `min(4, number of cores)`
/// threads"). One thread is also the fallback when the core count
/// cannot be observed.
const MAX_WALK_THREADS: usize = 4;

/// The cancellation flag of a scan or sync (SYS-003, ALG-053).
///
/// A plain `Arc<AtomicBool>` wrapper: the walker consults it once per
/// entry, and the owning layer flips it once from its cancel command.
/// It carries no data on purpose — scan results flow through the
/// walker's event callback, so there is nothing for the flag to
/// publish, and a bare flag is the cheapest primitive that can be
/// checked on every entry of a parallel walk. Cloning hands out another
/// handle to the same flag; `Default` and [`CancelToken::new`] produce
/// a not-cancelled token.
///
/// The orderings are the weakest correct pair: `cancel` publishes with
/// `Release` and `is_cancelled` observes with `Acquire`, so a check that
/// sees `true` sees every store that preceded the cancel call. Nothing
/// else in the walker synchronizes on this flag — the events a thread
/// already delivered before the flip are simply delivered — so no
/// stronger ordering (and no other synchronization) is warranted.
///
/// T-076's command layer (`src-tauri/src/scan.rs`) owns the plumbing
/// (the `scan_cancel` command flipping the token the running scan
/// holds through the managed slot); this type is the primitive that
/// plumbing uses. The sync pipeline's cancellation (SYS-003 covers
/// both) should reuse this type or relocate it, not grow a second
/// one.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    /// Creates a token whose walk is not cancelled.
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Marks the operation cancelled: every later per-entry check
    /// observes it and the walk quits as soon as possible. Calling this
    /// more than once, or after the operation finished, is harmless.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Whether [`CancelToken::cancel`] has been called on this token or
    /// any of its clones.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// The traversal parameters of a scan: the caller's view of the §6.5
/// settings keys `scan.max_depth` (D-04) and `scan.exclude_names`
/// (D-05).
///
/// Plain parameters, deliberately: this module never reads
/// `config.toml`, the settings table, or the database. The command
/// layer builds one of these per scan from the settings it already
/// loaded and validated (`max_depth` within 1–20, the exclusion list as
/// stored). There is no `Default`: defaulting the parameters here would
/// duplicate the settings layer's §6.5 defaults inside the walker, and
/// the two copies would drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    /// How many levels of folders below the root the walk visits (the
    /// root itself is always visited). Passed one for one to
    /// `WalkBuilder::max_depth`; the non-zero type rules out `ignore`'s
    /// unlimited-walk reading, so an unvalidated settings value cannot
    /// turn the scan into a full-drive traversal.
    pub max_depth: NonZeroUsize,
    /// Folder names to skip during the walk (exact name match,
    /// case-sensitive: the list is user data and the spec pins no case
    /// folding for it). The four ALG-051 system folders are skipped in
    /// addition to whatever is configured here.
    pub exclude_names: Vec<String>,
}

/// One thing the walk found, checked, or refused, delivered to the
/// consumer's callback.
///
/// The three variants are PRJ-002's scan-window facts: repositories
/// land in "New"/"Already added" (the classification layer's,
/// `scan_classify` + the command), plain directories count as
/// "folders checked", and skips land in
/// "Skipped" with the reason. Events own their paths, so the callback
/// may store paths without re-cloning on the walker's account — and
/// `&ScanEvent` delivery keeps read-only consumers (a progress counter)
/// allocation-free.
#[derive(Debug)]
pub enum ScanEvent {
    /// A folder containing a `.git` DIRECTORY: a found repository
    /// (ALG-052, D-06). The walk continues inside it; nested
    /// repositories arrive as their own events (ALG-051).
    Repository {
        /// The repository root (the folder holding `.git`).
        path: PathBuf,
    },
    /// A plain folder: inspected, no `.git` entry, no exclusion — the
    /// "folders checked" counter of PRJ-002. The walk descends into it.
    Directory {
        /// The folder's path.
        path: PathBuf,
    },
    /// A folder (or entry) the scan did not descend into, with the
    /// reason the UI shows (PRJ-002).
    Skipped {
        /// The skipped entry's path.
        path: PathBuf,
        /// Why it was skipped.
        reason: SkipReason,
    },
}

impl ScanEvent {
    /// The path this event is about (the repository root, the checked
    /// folder, or the skipped entry).
    pub fn path(&self) -> &Path {
        match self {
            ScanEvent::Repository { path } | ScanEvent::Directory { path } => path,
            ScanEvent::Skipped { path, .. } => path,
        }
    }
}

/// Why a scan skipped an entry (PRJ-002's skip-reason vocabulary).
///
/// The four variants are the walker's whole skip vocabulary; the
/// remaining PRJ-002 reasons ("archived or in exclusions") are the
/// classification task's, computed against the database, not facts of
/// the folder tree. `Access` carries the OS failure as its source so
/// §10's log facts (`raw_os_error`, `ErrorKind`) ride along untouched.
#[derive(Debug)]
pub enum SkipReason {
    /// The folder's name is excluded from the scan: on the caller's
    /// [`ScanOptions::exclude_names`], or one of the fixed ALG-051
    /// system folders. The UI shows both as "excluded" (PRJ-002), so
    /// they share one variant; the walk did not descend into the
    /// folder.
    Excluded,
    /// The folder has a `.git` entry that is not a directory — a file
    /// (the submodule or linked-worktree gitdir pointer) or a symlink
    /// to one (ALG-052, D-06). The folder is not auto-detected as a
    /// repository; the user can add it by hand. The walk did not
    /// descend into the folder.
    SubmoduleOrWorktree,
    /// The folder's path is not valid UTF-8 (the D-086 addendum, owner
    /// ruling 2026-11-20 variant 2): on a byte-preserving filesystem
    /// the folder's own name — or an ancestor's — carries bytes no
    /// TEXT identity can hold, so the path could never be keyed or
    /// stored losslessly; the identity seam refuses such paths at the
    /// add/scan boundary instead of keying them lossily (the
    /// pre-addendum U+FFFD behavior is deleted). The walk did not
    /// descend into the folder: every descendant's path inherits the
    /// invalid bytes. No OS call failed — a fact about the text, like
    /// `Excluded`, not an access refusal.
    NotUtf8,
    /// The OS refused to inspect the folder: access denied on the
    /// folder or an ancestor, the drive dropped, a listing failed
    /// mid-walk (PRJ-007). The traversal continued elsewhere. The
    /// source error keeps the OS code for the §10 log line and the UI
    /// detail. One vanish-shape caveat, stated honestly: a folder that
    /// disappears between its parent's listing and this walker's
    /// `.git` probe reports as a plain [`ScanEvent::Directory`] — the
    /// probe cannot distinguish "gone" from "no `.git`", and the
    /// vanish reaches the command layer as the last classification the
    /// folder had, not as an access skip.
    Access {
        /// The OS failure, reconstructed from `ignore`'s owned error:
        /// for OS-backed errors, the exact `raw_os_error` (and with it
        /// std's kind and strerror text); for the shapes with no OS
        /// error behind them (none is reachable with this walker's
        /// configuration — no ignore files are read and no links are
        /// followed), a synthetic error with the original's message.
        source: io::Error,
    },
}

impl SkipReason {
    /// The raw OS error code behind this skip, when an OS call failed
    /// (`raw_os_error` of the carried [`io::Error`]): PRJ-007's "with
    /// the OS code", consumed by the UI detail and the §10 log line.
    /// `None` for the skips that are facts about the name or text, not
    /// OS failures.
    pub fn os_code(&self) -> Option<i32> {
        match self {
            SkipReason::Excluded | SkipReason::SubmoduleOrWorktree | SkipReason::NotUtf8 => None,
            SkipReason::Access { source } => source.raw_os_error(),
        }
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::Excluded => write!(f, "folder name is excluded from the scan"),
            // ALG-052's own reason words, verbatim.
            SkipReason::SubmoduleOrWorktree => write!(f, "submodule or worktree"),
            SkipReason::NotUtf8 => write!(f, "path is not valid UTF-8"),
            SkipReason::Access { source } => write!(f, "folder cannot be read: {source}"),
        }
    }
}

impl std::error::Error for SkipReason {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SkipReason::Access { source } => Some(source),
            SkipReason::Excluded | SkipReason::SubmoduleOrWorktree | SkipReason::NotUtf8 => None,
        }
    }
}

/// How a scan ended: the walk ran to completion, or the cancel token
/// flipped and the walk quit ([`ScanOutcome::Cancelled`]).
///
/// Neither outcome says anything about per-entry failures — those are
/// [`ScanEvent::Skipped`] data, never function-level errors. The verdict
/// is exact, not sampled: `Cancelled` if and only if at least one walker
/// thread OBSERVED the flipped token on an entry and quit because of it,
/// `Completed` if no thread ever did. A flip that races with the last
/// entry (after every check, before the caller regains control) did not
/// interrupt anything, so it reports [`ScanOutcome::Completed`] — every
/// result is whole, and the command layer may treat the late cancel as
/// the no-op it was. Whether to keep or drop events after a
/// [`ScanOutcome::Cancelled`] is the command layer's rule (PRJ-002:
/// cancelling adds no projects).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanOutcome {
    /// Every reachable entry was visited (skips included); no walker
    /// thread observed the token flipped.
    Completed,
    /// At least one walker thread observed the [`CancelToken`] flipped
    /// on an entry and quit because of it.
    Cancelled,
}

impl ScanOutcome {
    /// Whether the scan was interrupted by cancellation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, ScanOutcome::Cancelled)
    }
}

/// Walks `root` for repositories, reporting every found, checked, or
/// skipped folder through `on_event`.
///
/// See the module documentation for the walk configuration (ALG-050),
/// the classification probe (ALG-052), the skip reasons (ALG-051,
/// PRJ-007), cancellation (ALG-053, SYS-003), and the unspecified event
/// ordering. The callback is called from several walker threads and
/// must therefore be [`Send`] + [`Sync`]; it receives `&ScanEvent`, so
/// counting consumers pay no allocation, and storing consumers clone
/// what they keep.
///
/// # Parameters, not configuration
///
/// `options` carries the caller's already-validated §6.5 settings; this
/// function reads no settings, no `config.toml`, and no database. The
/// scan root is exempt from the name filters: the user chose it
/// explicitly.
///
/// # Errors
///
/// None — the function is total. Per-entry failures are
/// [`ScanEvent::Skipped`] events with [`SkipReason::Access`]; a root
/// that cannot be inspected yields one such event and the walk ends.
pub fn scan_repositories(
    root: &Path,
    options: &ScanOptions,
    cancel: &CancelToken,
    on_event: impl Fn(&ScanEvent) + Send + Sync,
) -> ScanOutcome {
    // ALG-050: no ignore-file source is consulted, so a repository
    // under an ignored directory stays visible.
    let mut builder = WalkBuilder::new(root);
    let walker = builder
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        // The spec's only skip rules are the explicit excluded names
        // and system folders; a hidden filter would hide repositories
        // the spec wants found (see the module docs).
        .hidden(false)
        // D-07 / ALG-050: symlinks and Windows reparse points (which
        // std reports as symlinks) are never followed.
        .follow_links(false)
        .max_depth(Some(options.max_depth.get()))
        .threads(walk_thread_count());

    // Set by a visitor at the moment it answers `Quit`, so the outcome
    // reports exactly "a thread observed the flip", not the looser
    // "the token was flipped when the walk happened to end" (a flip
    // racing with the last entry must not cancel a completed walk).
    // Plain stack storage is sound: `run` blocks until every walker
    // thread has finished, so no reference outlives it.
    let quit_observed = AtomicBool::new(false);

    walker.build_parallel().run(|| {
        // One visitor per walker thread, all sharing the caller's
        // callback and the token behind shared references.
        let on_event = &on_event;
        let quit_observed = &quit_observed;
        Box::new(move |entry: Result<ignore::DirEntry, ignore::Error>| {
            visit_entry(cancel, quit_observed, options, on_event, entry)
        })
    });

    if quit_observed.load(Ordering::Acquire) {
        ScanOutcome::Cancelled
    } else {
        ScanOutcome::Completed
    }
}

/// Classifies one walk entry: the per-entry cancellation check, the
/// name filters, and the `.git` probe. Returns the walker's descent
/// instruction for the entry.
fn visit_entry(
    cancel: &CancelToken,
    quit_observed: &AtomicBool,
    options: &ScanOptions,
    on_event: &impl Fn(&ScanEvent),
    entry: Result<ignore::DirEntry, ignore::Error>,
) -> WalkState {
    // ALG-053 / SYS-003: checked on EVERY entry, before any other work
    // for it. `Quit` stops every walker thread promptly.
    if cancel.is_cancelled() {
        quit_observed.store(true, Ordering::Release);
        return WalkState::Quit;
    }

    match entry {
        // A walker-reported failure: a root that cannot be statted, a
        // directory whose listing failed (the searchable-but-not-
        // listable corner is described in the module docs). The path is
        // taken before the reason is built; every shape reachable with
        // this walker's configuration carries one.
        Err(error) => {
            let path = error_path(&error).map(Path::to_path_buf);
            // The io failure is reconstructed from `ignore`'s owned
            // error without ever moving or unwrapping it: OS-backed
            // errors keep their exact raw code (and with it std's kind
            // and message), the unreachable non-io shapes get a
            // synthetic error with the original's text.
            let reason = match error.io_error() {
                Some(io_error) => SkipReason::Access {
                    source: match io_error.raw_os_error() {
                        Some(code) => io::Error::from_raw_os_error(code),
                        None => io::Error::new(io_error.kind(), io_error.to_string()),
                    },
                },
                None => SkipReason::Access {
                    source: io::Error::other(error.to_string()),
                },
            };
            if let Some(path) = path {
                on_event(&ScanEvent::Skipped { path, reason });
            }
            WalkState::Skip
        }

        // Files and symlinks (including junctions, which std reports as
        // symlinks) are neither classified nor descended: D-07 says
        // links are not followed, and ALG-054 says files are never
        // opened. They produce no event — the scan counts folders, not
        // files. (`file_type()` is always known for entries this
        // configuration yields: roots are statted by the builder,
        // children carry the directory-entry type with a stat fallback;
        // the guard's else side merely declines to classify what cannot
        // be typed.)
        Ok(entry) if !entry.file_type().is_some_and(|ft| ft.is_dir()) => WalkState::Continue,

        Ok(entry) => {
            let path = entry.path();
            // The UTF-8 identity gate (the D-086 addendum), BEFORE the
            // name filters: a path whose text is not valid UTF-8 can
            // never become a registered identity — the key builder and
            // the canonicalizing add seam refuse it typed — so the walk
            // reports it as a skip and does not descend (every
            // descendant inherits the invalid bytes). Applied at every
            // depth INCLUDING the root: unlike the excluded-name policy
            // below, this is the TEXT boundary of the identity itself,
            // not a user choice the root could be exempt from.
            if path.to_str().is_none() {
                on_event(&ScanEvent::Skipped {
                    path: path.to_path_buf(),
                    reason: SkipReason::NotUtf8,
                });
                return WalkState::Skip;
            }
            // ALG-051: excluded names and system folders are skipped
            // and not descended — and not probed either, so exclusion
            // wins over repository detection (see the module docs).
            // The root (depth 0) is the user's explicit choice and is
            // exempt.
            if entry.depth() > 0 && is_excluded_name(path.file_name(), options) {
                on_event(&ScanEvent::Skipped {
                    path: path.to_path_buf(),
                    reason: SkipReason::Excluded,
                });
                return WalkState::Skip;
            }

            // ALG-052: the one classification probe. `symlink_metadata`
            // does not follow the `.git` entry if it is a link:
            // presence and kind, not validity (the D-084 idiom).
            match fs::symlink_metadata(path.join(".git")) {
                Ok(metadata) if metadata.is_dir() => {
                    on_event(&ScanEvent::Repository {
                        path: path.to_path_buf(),
                    });
                    // ALG-051: the scan continues inside a found
                    // repository, so nested repositories are found
                    // separately.
                    WalkState::Continue
                }
                // Any other `.git` kind — a file, or a symlink to one —
                // is the submodule/worktree form: skipped, not
                // descended (D-06: "you can add it by hand").
                Ok(_) => {
                    on_event(&ScanEvent::Skipped {
                        path: path.to_path_buf(),
                        reason: SkipReason::SubmoduleOrWorktree,
                    });
                    WalkState::Skip
                }
                Err(source) if source.kind() == io::ErrorKind::NotFound => {
                    on_event(&ScanEvent::Directory {
                        path: path.to_path_buf(),
                    });
                    WalkState::Continue
                }
                // PRJ-007: the OS refused the probe (denied search bit,
                // dropped drive). Skip with the OS code; the walk
                // continues with the siblings. Not descending: a folder
                // that refuses its own classification refuses its
                // contents too.
                Err(source) => {
                    on_event(&ScanEvent::Skipped {
                        path: path.to_path_buf(),
                        reason: SkipReason::Access { source },
                    });
                    WalkState::Skip
                }
            }
        }
    }
}

/// The path an `ignore` error is attached to, when it carries one. The
/// reachable shapes (a root that cannot be statted, a failed directory
/// listing) wrap their failures in `WithPath`; the recursion is total
/// over the wrapper variants so a future `ignore` shape degrades to "no
/// event" instead of a wrong path.
fn error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithDepth { err, .. } => error_path(err),
        ignore::Error::WithLineNumber { err, .. } => error_path(err),
        ignore::Error::Partial(errors) => errors.iter().find_map(error_path),
        _ => None,
    }
}

/// Whether a folder NAME is skipped by the scan: on the caller's
/// exclusion list, or one of the fixed system folders (ALG-051). The
/// comparison is exact and case-sensitive — the list is user data and
/// the spec pins no case folding; the system names are matched as the
/// platforms spell them. A name-less path (the filesystem root) is
/// never excluded.
fn is_excluded_name(name: Option<&std::ffi::OsStr>, options: &ScanOptions) -> bool {
    let Some(name) = name else {
        return false;
    };
    options
        .exclude_names
        .iter()
        .map(String::as_str)
        .chain(SYSTEM_FOLDER_NAMES.iter().copied())
        .any(|excluded| std::ffi::OsStr::new(excluded) == name)
}

/// `min(4, available cores)`, per ALG-050's thread cap; one thread when
/// the core count cannot be observed.
fn walk_thread_count() -> usize {
    std::thread::available_parallelism().map_or(1, |cores| cores.get().min(MAX_WALK_THREADS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use tempfile::TempDir;

    /// A flat, order-free summary of one event, so assertions are on
    /// SETS of facts: the parallel walk's delivery order is not
    /// specified (see the module docs) and must never be depended on.
    #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
    enum Recorded {
        Repository(String),
        Directory(String),
        SkippedExcluded(String),
        SkippedSubmoduleOrWorktree(String),
        SkippedNotUtf8(String),
        SkippedAccess(String),
    }

    impl Recorded {
        fn path_contains(&self, needle: &str) -> bool {
            match self {
                Recorded::Repository(path)
                | Recorded::Directory(path)
                | Recorded::SkippedExcluded(path)
                | Recorded::SkippedSubmoduleOrWorktree(path)
                | Recorded::SkippedNotUtf8(path)
                | Recorded::SkippedAccess(path) => path.contains(needle),
            }
        }

        fn is_repository_of(&self, path: &Path) -> bool {
            matches!(self, Recorded::Repository(p) if *p == path.to_string_lossy())
        }
    }

    /// Runs a scan of `root` and records every event, sorted, so the
    /// assertions never depend on delivery order.
    fn record_all(
        root: &Path,
        max_depth: usize,
        exclude_names: &[&str],
    ) -> (ScanOutcome, Vec<Recorded>) {
        let options = ScanOptions {
            max_depth: NonZeroUsize::new(max_depth).expect("test depths are non-zero"),
            exclude_names: exclude_names.iter().map(|name| name.to_string()).collect(),
        };
        let cancel = CancelToken::new();
        let events = std::sync::Mutex::new(Vec::new());
        let outcome = scan_repositories(root, &options, &cancel, |event| {
            let recorded = match event {
                ScanEvent::Repository { path } => Recorded::Repository(path_string(path)),
                ScanEvent::Directory { path } => Recorded::Directory(path_string(path)),
                ScanEvent::Skipped { path, reason } => match reason {
                    SkipReason::Excluded => Recorded::SkippedExcluded(path_string(path)),
                    SkipReason::SubmoduleOrWorktree => {
                        Recorded::SkippedSubmoduleOrWorktree(path_string(path))
                    }
                    SkipReason::NotUtf8 => Recorded::SkippedNotUtf8(path_string(path)),
                    SkipReason::Access { .. } => Recorded::SkippedAccess(path_string(path)),
                },
            };
            events
                .lock()
                .expect("the test collector cannot be poisoned")
                .push(recorded);
        });
        let mut events = events
            .into_inner()
            .expect("the test collector cannot be poisoned");
        events.sort();
        (outcome, events)
    }

    fn path_string(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    /// Creates `dir/name/.git` as a directory (ALG-052's repository
    /// form) and returns the repository's path.
    fn repo_with_git_dir(dir: &Path, name: &str) -> PathBuf {
        let repo = dir.join(name);
        fs::create_dir_all(repo.join(".git")).expect("creating the fixture repository must work");
        repo
    }

    /// Creates `dir/name/.git` as a FILE holding a gitdir pointer
    /// (ALG-052's submodule/worktree form) and returns the folder's
    /// path.
    fn folder_with_git_file(dir: &Path, name: &str) -> PathBuf {
        let folder = dir.join(name);
        fs::create_dir_all(&folder).expect("creating the fixture folder must work");
        fs::write(folder.join(".git"), "gitdir: ../elsewhere/repo/.git\n")
            .expect("writing the gitdir pointer must work");
        folder
    }

    /// The fixture's owner uid is this process's effective uid (the
    /// fixture was created by it): uid 0 means the tests run as root,
    /// where mode bits are not enforced and a chmod-000 directory stays
    /// readable — the precondition of a permission test cannot be set
    /// up. The caller skips loudly instead of passing vacuously
    /// (precedent: `paths.rs`, and `running_as_root_is_untestable` in
    /// eunomia-store).
    #[cfg(unix)]
    fn running_as_root_is_untestable(temp: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(temp)
            .expect("the fixture directory must be statable")
            .uid()
            == 0
    }

    #[test]
    fn nested_repositories_are_found_separately() {
        // The acceptance pair (a): an outer repository and a repository
        // nested inside it are BOTH found, which proves the walk
        // continued inside the found outer repository (ALG-051).
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let outer = repo_with_git_dir(dir.path(), "outer");
        let inner = repo_with_git_dir(&outer, "inner");
        fs::create_dir_all(outer.join("plain"))
            .expect("creating a plain folder inside the repository must work");

        let (outcome, events) = record_all(dir.path(), 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        let repositories: HashSet<&str> = events
            .iter()
            .filter_map(|event| match event {
                Recorded::Repository(path) => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            repositories,
            HashSet::from([
                outer.to_string_lossy().as_ref(),
                inner.to_string_lossy().as_ref()
            ]),
            "outer and nested repositories must both be found; got: {events:?}"
        );
    }

    #[test]
    fn a_git_file_is_skipped_with_the_exact_reason() {
        // ALG-052 / D-06: `.git` as a FILE (submodule or linked
        // worktree) is a skip with the spec's reason, and the folder is
        // NOT descended into — a repository nested inside it is not
        // found (the user adds the folder by hand).
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let worktree = folder_with_git_file(dir.path(), "worktree");
        repo_with_git_dir(&worktree, "nested");

        let (outcome, events) = record_all(dir.path(), 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::SkippedSubmoduleOrWorktree(
                worktree.to_string_lossy().into_owned()
            )),
            "the .git-file folder must be skipped as submodule or worktree; got: {events:?}"
        );
        assert!(
            !events.iter().any(|event| event.path_contains("nested")),
            "a skipped submodule/worktree folder must not be descended into; got: {events:?}"
        );
    }

    /// Linux-only, like the sweep test's non-UTF-8 fixture (the
    /// `temp_files` platform posture): only Linux filesystems accept
    /// an invalid-byte name — APFS and NTFS reject it with EILSEQ — so
    /// the D-086-addendum walk gate is testable only here.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_non_utf8_scan_root_yields_one_skip_and_completes() {
        // The gate's ROOT arm, the one claim of the module docs no
        // other fixture reaches: a root whose own path is not valid
        // UTF-8 is skipped like any other entry (the identity
        // boundary is not a name policy the root could be exempt
        // from), the walk cannot descend, and it ends Completed with
        // exactly one event — data, not a walker failure.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join(OsStr::from_bytes(b"scan\xff"));
        fs::create_dir_all(root.join(".git"))
            .expect("the non-UTF-8 scan-root fixture must be creatable");

        let (outcome, events) = record_all(&root, 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert_eq!(
            events,
            vec![Recorded::SkippedNotUtf8(
                root.to_string_lossy().into_owned()
            )],
            "a non-UTF-8 root yields exactly one NotUtf8 skip: {events:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_non_utf8_repository_is_skipped_and_its_subtree_is_not_walked() {
        // The walk's half of the D-086 addendum: a repository whose
        // path text is not valid UTF-8 (its own name here, an
        // ancestor's name in the second fixture) is NEVER a found
        // repository — it is a skip with the NotUtf8 reason, the
        // subtree under the invalid component is not walked, and the
        // valid siblings are found untouched.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let non_utf8 = dir.path().join(OsStr::from_bytes(b"repo\xff"));
        fs::create_dir_all(non_utf8.join(".git"))
            .expect("the non-UTF-8 repository fixture must be creatable");
        // A VALID-named repository under a non-UTF-8-named ancestor:
        // its whole path carries the ancestor's invalid bytes, so the
        // ancestor is skipped and the walk never reaches the child.
        let nested_parent = dir.path().join(OsStr::from_bytes(b"dir\xfe"));
        let nested = nested_parent.join("inner");
        fs::create_dir_all(nested.join(".git"))
            .expect("the non-UTF-8 ancestor fixture must be creatable");
        let sibling = repo_with_git_dir(dir.path(), "sibling");

        let (outcome, events) = record_all(dir.path(), 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::SkippedNotUtf8(
                non_utf8.to_string_lossy().into_owned()
            )),
            "the non-UTF-8-named repository must be skipped with the NotUtf8 \
             reason; got: {events:?}"
        );
        assert!(
            events.contains(&Recorded::SkippedNotUtf8(
                nested_parent.to_string_lossy().into_owned()
            )),
            "the non-UTF-8-named ANCESTOR must be skipped too — every path \
             below it is unregistrable; got: {events:?}"
        );
        assert!(
            !events.iter().any(|event| event.is_repository_of(&non_utf8)),
            "a non-UTF-8 repository must never be REPORTED as one — no lossy \
             find, no later lossy classification; got: {events:?}"
        );
        assert!(
            !events.iter().any(|event| event.path_contains("inner")),
            "the walk must not descend into a non-UTF-8 path; got: {events:?}"
        );
        assert!(
            events.contains(&Recorded::Repository(
                sibling.to_string_lossy().into_owned()
            )),
            "the valid sibling repository is still found — valid paths are \
             untouched by the gate; got: {events:?}"
        );
    }

    #[test]
    fn excluded_names_are_skipped_and_not_descended() {
        // D-05 / ALG-051: a folder named on the exclusion list is
        // skipped with the "excluded" reason, is not descended into,
        // and — the precedence ruling — is not classified even when it
        // holds a `.git` directory: exclusion wins over detection.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let vendor = repo_with_git_dir(dir.path(), "vendor");
        let plain = dir.path().join("src");
        fs::create_dir_all(plain.join("deep")).expect("creating the fixture tree must work");

        let (outcome, events) = record_all(dir.path(), 6, &["vendor"]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::SkippedExcluded(
                vendor.to_string_lossy().into_owned()
            )),
            "the excluded folder must be skipped with the excluded reason; got: {events:?}"
        );
        assert!(
            !events.iter().any(|event| event.is_repository_of(&vendor)),
            "an excluded folder must not be classified as a repository even with .git; got: {events:?}"
        );
        assert!(
            !events.iter().any(|event| event.path_contains(&format!(
                "{}{}",
                vendor.display(),
                std::path::MAIN_SEPARATOR
            ))),
            "nothing under the excluded folder may be visited: the walk must not \
             descend into it; got: {events:?}"
        );
        assert!(
            events.contains(&Recorded::Directory(plain.to_string_lossy().into_owned()))
                && events.contains(&Recorded::Directory(
                    plain.join("deep").to_string_lossy().into_owned()
                )),
            "unlisted folders must still be walked and reported; got: {events:?}"
        );
    }

    #[test]
    fn system_folders_are_skipped() {
        // ALG-051's fixed list, spelled exactly as the spec names them.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        for name in SYSTEM_FOLDER_NAMES {
            fs::create_dir_all(dir.path().join(name))
                .expect("creating the system-folder fixture must work");
        }

        let (outcome, events) = record_all(dir.path(), 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        for name in SYSTEM_FOLDER_NAMES {
            let path = dir.path().join(name).to_string_lossy().into_owned();
            assert!(
                events.contains(&Recorded::SkippedExcluded(path)),
                "{name} must be skipped as a system folder with no configured names; got: {events:?}"
            );
        }
    }

    #[test]
    fn the_depth_bound_is_respected() {
        // D-04: max_depth levels below the root. Depth 1 visits the
        // root and its direct children; grandchildren are never listed,
        // so a repository two levels down is not found.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let near = repo_with_git_dir(dir.path(), "near");
        let far_parent = dir.path().join("plain");
        let far = repo_with_git_dir(&far_parent, "deep");

        let (outcome, events) = record_all(dir.path(), 1, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::Repository(near.to_string_lossy().into_owned())),
            "a repository at depth 1 must be found; got: {events:?}"
        );
        assert!(
            events.contains(&Recorded::Directory(
                far_parent.to_string_lossy().into_owned()
            )),
            "the depth-1 folder must be visited and reported; got: {events:?}"
        );
        assert!(
            !events.iter().any(|event| event.is_repository_of(&far)),
            "a repository at depth 2 must not be found with max_depth 1; got: {events:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() {
        // D-07 / ALG-050: a symlinked folder is neither traversed nor
        // classified — a repository OUTSIDE the root, reachable only
        // through the link, is not found, and the link itself produces
        // no event.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let outside = TempDir::new().expect("a temporary directory must be creatable");
        let external_repo = repo_with_git_dir(outside.path(), "external");
        std::os::unix::fs::symlink(&external_repo, dir.path().join("link"))
            .expect("creating the fixture symlink must work");

        let (outcome, events) = record_all(dir.path(), 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            !events
                .iter()
                .any(|event| event.is_repository_of(&external_repo)),
            "a repository behind a symlink must not be traversed; got: {events:?}"
        );
        assert!(
            !events.iter().any(|event| event.path_contains("link")),
            "a symlinked folder must produce no event; got: {events:?}"
        );
    }

    #[test]
    fn gitignore_rules_are_not_consulted() {
        // ALG-050: no ignore-file source is read — a repository under a
        // `.gitignore`d directory must stay visible.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let outer = repo_with_git_dir(dir.path(), "outer");
        fs::write(outer.join(".gitignore"), "hidden/\n")
            .expect("writing the fixture .gitignore must work");
        let hidden_repo = repo_with_git_dir(&outer, "hidden");

        let (outcome, events) = record_all(dir.path(), 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::Repository(
                hidden_repo.to_string_lossy().into_owned()
            )),
            "a repository under a .gitignore'd directory must be found: the walk \
             must not consult ignore files; got: {events:?}"
        );
    }

    #[test]
    fn no_ignore_source_or_hidden_filter_thins_the_walk() {
        // ALG-050's remaining three switches, each pinned so a one-flag
        // regression cannot silently hide repositories (the failure
        // mode the "no ignore rule may thin the scan" claim forbids):
        // (a) `.ignore` files are not read (a DIFFERENT switch from
        // `.gitignore`); (b) the hidden filter is off — a repository in
        // a dot-named folder is found; (c) parent-directory rules above
        // the scan root are not consulted.
        let dir = TempDir::new().expect("a temporary directory must be creatable");

        // (a) a `.ignore` file hiding the fixture name.
        fs::write(dir.path().join(".ignore"), "ignored-by-dot-ignore/\n")
            .expect("writing the fixture .ignore must work");
        let dot_ignore_repo = repo_with_git_dir(&dir.path().join("ignored-by-dot-ignore"), "repo");

        // (b) a repository inside a dot-named folder.
        let dot_folder_repo = repo_with_git_dir(&dir.path().join(".old"), "repo");

        // (c) a rule in the PARENT of the scan root, hiding a folder
        // inside it. What this pins is the spec-level claim "no ignore
        // rule above the root thins the walk": with every ignore-file
        // source off, `ignore` reads no parent rules at all (its
        // `add_parents` is gated on the parent-sourced flags), so the
        // repository under a parent-level rule stays found. The
        // `parents` flag alone is inert while the other sources are
        // off — flipping only it cannot make this assertion fail; the
        // failing scenario is any future configuration that re-enables
        // a parent-sourced source (round-2 review F-2).
        let outer = dir.path().join("outer");
        let scan_root = outer.join("scan_root");
        fs::create_dir_all(&scan_root).expect("creating the fixture tree must work");
        fs::write(outer.join(".gitignore"), "inner/\n")
            .expect("writing the parent .gitignore must work");
        let parent_rule_repo = repo_with_git_dir(&scan_root.join("inner"), "repo");

        let (outcome, events) = record_all(&scan_root, 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::Repository(
                parent_rule_repo.to_string_lossy().into_owned()
            )),
            "a repository under a parent-level .gitignore rule must be found: \
             with all ignore sources off, no parent rule is read; got: {events:?}"
        );

        let (outcome, events) = record_all(dir.path(), 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::Repository(
                dot_ignore_repo.to_string_lossy().into_owned()
            )),
            "a repository under a .ignore rule must be found: .ignore files are \
             not read; got: {events:?}"
        );
        assert!(
            events.contains(&Recorded::Repository(
                dot_folder_repo.to_string_lossy().into_owned()
            )),
            "a repository in a dot-named folder must be found: the hidden filter \
             is off (ALG-051's explicit names are the only skip rules); got: {events:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_root_is_traversed_as_the_folder_it_names() {
        // The documented exception to "symlinks are not followed" (D-07
        // covers links BELOW the root): the root is resolved by the
        // builder with links followed — a root that is a symlink is
        // scanned as the repository it names, like the availability
        // check accepts a symlinked project root. Pinned so an `ignore`
        // upgrade cannot change it unnoticed.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let real = repo_with_git_dir(dir.path(), "real");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("creating the root symlink must work");

        let (outcome, events) = record_all(&link, 6, &[]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::Repository(link.to_string_lossy().into_owned())),
            "a symlinked root must be classified as the repository it names; got: {events:?}"
        );
    }

    #[test]
    fn the_root_is_classified_and_bypasses_the_name_filters() {
        // The scan root is the user's explicit choice: its name never
        // filters it, and it is classified like any folder — a root
        // named like an exclusion and holding `.git` is a found
        // repository, and the walk descends into it.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = repo_with_git_dir(dir.path(), "vendor");
        let nested = repo_with_git_dir(&root, "inner");

        let (outcome, events) = record_all(&root, 6, &["vendor"]);

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::Repository(root.to_string_lossy().into_owned())),
            "the root must be classified despite its excluded-looking name; got: {events:?}"
        );
        assert!(
            events.contains(&Recorded::Repository(nested.to_string_lossy().into_owned())),
            "the walk must descend into the root repository; got: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Recorded::SkippedExcluded(_))),
            "no root-named exclusion may fire; got: {events:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_does_not_stop_the_traversal() {
        // PRJ-007 and the acceptance pair (b): a folder without read
        // rights is skipped, and its SIBLINGS are still found.
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let locked = dir.path().join("locked");
        fs::create_dir_all(&locked).expect("creating the fixture folder must work");
        let repo = repo_with_git_dir(dir.path(), "repo");
        if running_as_root_is_untestable(dir.path()) {
            eprintln!(
                "skipping an_unreadable_folder_does_not_stop_the_traversal: \
                 running as root — mode bits are not enforced, the \
                 unreadable-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let (outcome, events) = record_all(dir.path(), 6, &[]);
        // Restore before asserting, so a panic cannot strand an
        // unreadable fixture under TempDir's teardown.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::SkippedAccess(
                locked.to_string_lossy().into_owned()
            )),
            "the unreadable folder must be skipped with an access reason; got: {events:?}"
        );
        assert!(
            events.contains(&Recorded::Repository(repo.to_string_lossy().into_owned())),
            "the sibling repository must still be found: the refusal must not \
             stop the traversal; got: {events:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_access_skip_carries_the_os_code() {
        // PRJ-007's "with the OS code": EACCES (13) rides on the skip.
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let locked = dir.path().join("locked");
        fs::create_dir_all(&locked).expect("creating the fixture folder must work");
        if running_as_root_is_untestable(dir.path()) {
            eprintln!(
                "skipping an_access_skip_carries_the_os_code: running as \
                 root — mode bits are not enforced, the unreadable-folder \
                 precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let options = ScanOptions {
            max_depth: NonZeroUsize::new(6).expect("6 is non-zero"),
            exclude_names: Vec::new(),
        };
        let cancel = CancelToken::new();
        let codes = std::sync::Mutex::new(Vec::new());
        let outcome = scan_repositories(dir.path(), &options, &cancel, |event| {
            if let ScanEvent::Skipped { reason, .. } = event {
                codes
                    .lock()
                    .expect("the test collector cannot be poisoned")
                    .push((path_string(event.path()), reason.os_code()));
            }
        });
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        assert_eq!(outcome, ScanOutcome::Completed);
        let codes = codes
            .into_inner()
            .expect("the test collector cannot be poisoned");
        assert!(
            codes
                .iter()
                .any(|(path, code)| *path == locked.to_string_lossy() && *code == Some(13)),
            "the unreadable folder's skip must carry EACCES = 13; got: {codes:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_root_yields_one_skip_and_completes() {
        // The root-refusal path: the walk reports the root skipped with
        // an access reason and ends cleanly — a refused root is data,
        // not a walker failure.
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("root");
        fs::create_dir_all(&root).expect("creating the fixture root must work");
        if running_as_root_is_untestable(dir.path()) {
            eprintln!(
                "skipping an_unreadable_root_yields_one_skip_and_completes: \
                 running as root — mode bits are not enforced, the \
                 unreadable-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&root, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let (outcome, events) = record_all(&root, 6, &[]);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        assert_eq!(outcome, ScanOutcome::Completed);
        let access_skips: Vec<&Recorded> = events
            .iter()
            .filter(|event| matches!(event, Recorded::SkippedAccess(_)))
            .collect();
        assert_eq!(
            access_skips.len(),
            1,
            "an unreadable root must produce exactly one access skip; got: {events:?}"
        );
        assert!(
            access_skips[0].path_contains("root"),
            "the skip must be about the root itself; got: {events:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_searchable_but_unlistable_folder_is_checked_and_skipped() {
        // The documented two-event corner: with mode 0o111 (search
        // without read) the `.git` probe still succeeds, so the folder
        // is classified AND its failed listing arrives as an access
        // skip. Both facts are true and both must be delivered.
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let odd = dir.path().join("odd");
        fs::create_dir_all(&odd).expect("creating the fixture folder must work");
        let repo = repo_with_git_dir(dir.path(), "repo");
        if running_as_root_is_untestable(dir.path()) {
            eprintln!(
                "skipping a_searchable_but_unlistable_folder_is_checked_and_skipped: \
                 running as root — mode bits are not enforced, the \
                 unlistable-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&odd, fs::Permissions::from_mode(0o111))
            .expect("removing the read bit must work");
        let (outcome, events) = record_all(dir.path(), 6, &[]);
        fs::set_permissions(&odd, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::Directory(odd.to_string_lossy().into_owned())),
            "the probe (search bit) must classify the folder; got: {events:?}"
        );
        assert!(
            events.contains(&Recorded::SkippedAccess(odd.to_string_lossy().into_owned())),
            "the failed listing must arrive as an access skip; got: {events:?}"
        );
        assert!(
            events.contains(&Recorded::Repository(repo.to_string_lossy().into_owned())),
            "the sibling repository must still be found; got: {events:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_never_reported() {
        // ALG-054's observable half: the scan never opens files, so a
        // file that cannot even be READ produces no event and no
        // failure — only folders are classified. (The syscall-negative
        // cannot be asserted; the API shape pins it: no event variant
        // could carry file contents.)
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_dir(dir.path(), "repo");
        let file = repo.join("secret.txt");
        fs::write(&file, b"contents").expect("writing the fixture file must work");
        if running_as_root_is_untestable(&repo) {
            eprintln!(
                "skipping an_unreadable_file_is_never_reported: running as \
                 root — mode bits are not enforced, the unreadable-file \
                 precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&file, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let (outcome, events) = record_all(&repo, 6, &[]);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644))
            .expect("restoring mode bits must work");

        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            events.contains(&Recorded::Repository(repo.to_string_lossy().into_owned())),
            "the repository itself must be found; got: {events:?}"
        );
        assert!(
            !events.iter().any(|event| event.path_contains("secret.txt")),
            "a file must produce no event, readable or not; got: {events:?}"
        );
    }

    #[test]
    fn skip_reason_texts_and_os_codes_follow_the_contract() {
        // The command layer branches on the variants, shows the reason,
        // and logs the OS code; these assertions pin the Display texts
        // (lowercase, no trailing punctuation — the house style;
        // ALG-052's reason words verbatim) and the os_code()/source()
        // contract.
        let excluded = SkipReason::Excluded;
        assert_eq!(
            excluded.to_string(),
            "folder name is excluded from the scan"
        );
        assert_eq!(excluded.os_code(), None, "a name skip has no OS code");

        let submodule = SkipReason::SubmoduleOrWorktree;
        assert_eq!(submodule.to_string(), "submodule or worktree");
        assert_eq!(submodule.os_code(), None, "a name skip has no OS code");

        let not_utf8 = SkipReason::NotUtf8;
        assert_eq!(not_utf8.to_string(), "path is not valid UTF-8");
        assert_eq!(not_utf8.os_code(), None, "a text-fact skip has no OS code");
        assert!(
            std::error::Error::source(&not_utf8).is_none(),
            "a text-fact skip has no error source"
        );

        let access = SkipReason::Access {
            source: io::Error::from_raw_os_error(13),
        };
        assert!(
            access.to_string().starts_with("folder cannot be read: "),
            "the Access text must state the fact, then the OS text: {access}"
        );
        assert_eq!(
            access.os_code(),
            Some(13),
            "EACCES must ride through os_code()"
        );
        assert!(
            std::error::Error::source(&access).is_some(),
            "an access skip chains to its io error"
        );
        assert!(
            std::error::Error::source(&excluded).is_none()
                && std::error::Error::source(&submodule).is_none(),
            "name skips have no error source"
        );
    }

    #[test]
    fn a_cancelled_token_yields_no_events() {
        // The acceptance pair (c), deterministic half: a token flipped
        // before the walk starts is checked on the very first entry, so
        // nothing is ever delivered and the walk reports Cancelled.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        repo_with_git_dir(dir.path(), "repo");

        let options = ScanOptions {
            max_depth: NonZeroUsize::new(6).expect("6 is non-zero"),
            exclude_names: Vec::new(),
        };
        let cancel = CancelToken::new();
        cancel.cancel();
        let events = std::sync::Mutex::new(Vec::new());
        let outcome = scan_repositories(dir.path(), &options, &cancel, |event| {
            events
                .lock()
                .expect("the test collector cannot be poisoned")
                .push(path_string(event.path()));
        });

        assert_eq!(outcome, ScanOutcome::Cancelled);
        assert!(
            events
                .into_inner()
                .expect("the test collector cannot be poisoned")
                .is_empty(),
            "a pre-cancelled walk must yield no events"
        );
    }

    #[test]
    fn a_token_flipped_mid_walk_interrupts_the_traversal() {
        // The acceptance pair (c), mid-walk half: the flip lands inside
        // the callback for the FIRST yielded entry, so every later
        // per-entry check sees it. Determinism: each walker thread
        // checks the token before each entry, and a thread's delivery
        // is synchronous, so at flip time at most (threads - 1) other
        // entries have passed their check — strictly fewer than the
        // fixture's 201 folders. Reaching 201 deliveries would prove
        // the per-entry check does not exist; run() returning proves
        // termination is observable.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        const FOLDER_COUNT: usize = 200;
        for index in 0..FOLDER_COUNT {
            fs::create_dir_all(dir.path().join(format!("folder{index:03}")))
                .expect("creating the fixture tree must work");
        }

        let options = ScanOptions {
            max_depth: NonZeroUsize::new(6).expect("6 is non-zero"),
            exclude_names: Vec::new(),
        };
        let cancel = CancelToken::new();
        let delivered = std::sync::Mutex::new(0_usize);
        let flipped = std::sync::Mutex::new(false);
        let outcome = scan_repositories(dir.path(), &options, &cancel, |_event| {
            *delivered
                .lock()
                .expect("the test counter cannot be poisoned") += 1;
            let mut flipped = flipped.lock().expect("the test flag cannot be poisoned");
            if !*flipped {
                *flipped = true;
                cancel.cancel();
            }
        });

        assert_eq!(outcome, ScanOutcome::Cancelled);
        let delivered = delivered
            .into_inner()
            .expect("the test counter cannot be poisoned");
        assert!(
            delivered > 0,
            "the first entry must have been delivered before the flip"
        );
        // The exact per-entry-check bound: a delivery happens only
        // after its check passed, deliveries are synchronous, and the
        // flip lands inside the first one — so each of the
        // `walk_thread_count()` walker threads can deliver at most its
        // one in-flight entry after (or while) the flip lands, and its
        // very next check fails. Reaching the fixture's full count
        // would prove the per-entry check does not exist; run()
        // returning proves termination is observable.
        assert!(
            delivered <= walk_thread_count(),
            "the per-entry check must cut the walk short at the thread \
             count: got {delivered} delivered, threads = {}",
            walk_thread_count()
        );
        assert!(
            delivered < FOLDER_COUNT + 1,
            "the walk must not have delivered every folder: got {delivered} of \
             {} folders",
            FOLDER_COUNT + 1
        );
    }
}
