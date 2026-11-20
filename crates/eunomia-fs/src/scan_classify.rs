//! Classification of scanned repositories (docs/spec.md 9.6,
//! ALG-053; the scan window of PRJ-002/005/006).
//!
//! [`scan`](crate::scan) walks a folder and reports what it found;
//! THIS module is the second half of ALG-053: for every found
//! repository it computes the `path_key` ([`paths::path_key`],
//! DB-030) and the class — [`ScanClass::New`], [`ScanClass::Known`]
//! (active), [`ScanClass::Archived`], [`ScanClass::Excluded`] — by
//! looking the key up in a registry snapshot the CALLER supplies.
//! PRJ-002's scan window is the consumer: `new` feeds the "New"
//! group with its checkboxes (PRJ-005), `known` the "Already
//! added" group, and `archived`/`excluded` the "Skipped" group with
//! the reason "archived or in exclusions"; plain folders feed the
//! "folders checked" counter, and the walker's own skips pass
//! through with their reason. The command layer (the scan command
//! task) drives
//! [`scan_repositories`](crate::scan::scan_repositories), runs each
//! event through [`classify_entry`], and serializes the result into
//! the throttled progress event (SYS-003's `scan://progress`, at
//! most 10 per second) — the throttle and the cancellation are that
//! layer's and [`crate::scan`]'s; this module is synchronous,
//! per-entry, and pure.
//!
//! # The registry is a parameter, not a dependency
//!
//! The three membership sets — the `path_key`s of active projects,
//! archived projects, and `project_exclusions` rows — arrive in a
//! [`ScanRegistry`] built by the caller. This crate never reads the
//! database: the store layer owns those rows (D-093), and the seam
//! between them is exactly this snapshot. Passing it per scan also
//! makes the classification deterministic over one scan's lifetime:
//! adds and archives committed while a scan runs cannot flip a
//! class mid-walk — the next scan sees them.
//!
//! # The key, and the caller's canonical-root obligation (DB-030, D-086)
//!
//! [`classify_repository`] keys the path with the pure
//! [`paths::path_key`] — no canonicalization happens here, no
//! filesystem is touched: classification is a text-key lookup.
//! That is the DESIGN (D-086 names the scan a caller of the pure
//! key), and it puts ONE obligation on the caller, stated here
//! because the walker does not carry it: the scan ROOT must be
//! canonicalized — [`canonicalize`](crate::paths::canonicalize)
//! before
//! [`scan_repositories`](crate::scan::scan_repositories), the same
//! function whose output the add flow stores, so both sides of the
//! lookup spell the identity the same way. The walker itself
//! accepts any root verbatim, and a symlinked root is a documented,
//! deliberately supported input that is traversed as the folder it
//! names while the reported paths KEEP the link spelling
//! (`scan`'s own root test pins that) — every path below such a
//! root carries the link prefix and keys by its text, so a
//! registered repository behind it would classify `new` against
//! its canonical registry key. The command layer that owns the
//! root choice (the scan command task) owns this obligation, the
//! same way it owns building [`ScanRegistry`]; a pinned test in
//! this module documents the failure mode so the obligation cannot
//! be assumed away. A path that is not canonical still classifies,
//! by the key of its text; canonical aliasing (DB-031) is the ADD
//! flow's
//! [`canonical_path_key`](crate::paths::canonical_path_key), never
//! the classifier's. Two spellings that collapse to one key (a
//! doubled separator; NFC/NFD under the folding rules of Windows
//! and macOS) are one repository with one class.
//!
//! # Overlapping sets: a priority, not an error (PRJ-017, DB-015)
//!
//! ALG-053 does not say what to do when a key sits in two sets at
//! once, and the honest answer is that overlaps are not anomalies:
//! archiving a scan-found project KEEPS the archived row and WRITES
//! an exclusion for the same key (§6.4, PRJ-017; D-093 pins the
//! coexistence with an integration test), so `archived ∩ excluded`
//! is the steady state of every archived scan-found project. A
//! typed error would turn the normal case into a failure path.
//! Classification therefore resolves every overlap with one fixed
//! priority:
//!
//! 1. [`ScanClass::Known`] — the key is an ACTIVE project: the
//!    truest fact ("Already added"; DB-015 keeps one row per key,
//!    so active-vs-archived cannot both be rows, and a leftover
//!    exclusion cannot outrank a restored project — D-10's restore
//!    removes it).
//! 2. [`ScanClass::Archived`] — an archived project row wins over
//!    its exclusion side effect: the project still exists in the
//!    app, and DB-015's "offer to restore" flow is the answer, not
//!    the exclusions screen.
//! 3. [`ScanClass::Excluded`] — only keys with NO project row: the
//!    user removed the scan result or excluded the path by hand
//!    (`removed_scanned` / `manual`).
//!
//! [`ScanClass::New`] is the fall-through: the registry has no
//! project and no exclusion for the key.
//!
//! # Two different "excluded" facts
//!
//! The word names two unrelated things, and the module keeps them
//! apart: [`SkipReason::Excluded`] is the WALK's fact — a folder
//! NAME on the exclusion list or a system folder (ALG-051), skipped
//! before any classification, never a repository event at all —
//! while [`ScanClass::Excluded`] is the REGISTRY's fact — a FOUND
//! repository whose `path_key` is in `project_exclusions`. The
//! first rides a [`ScanEvent::Skipped`] unchanged; the second is a
//! classification of a [`ScanEvent::Repository`].
//!
//! # The result shapes
//!
//! Two shapes, two consumers, both owned here so the command layer
//! invents neither. [`ClassifiedRepository`] is OWNED (path, key,
//! class): found repositories are what the user acts on — the New
//! checkboxes, "Add selected", the restore offer — so they must
//! survive the scan callback. [`ClassifiedScanEvent`] is the
//! BORROWED envelope of [`classify_entry`]: a 1:1 mapping of every
//! [`ScanEvent`] where repositories gain their key and class and
//! everything else passes through by reference (`&Path`, `&
//! SkipReason`) — zero allocation for the folders-checked counter
//! and the skip display, which is most events, and a shape ready to
//! serialize into the progress event as-is.
//!
//! # Testing plan
//!
//! Everything goes through the public API. The acceptance pair is
//! end-to-end: a real fixture (two repositories plus a nested one)
//! scanned twice through
//! [`scan_repositories`](crate::scan::scan_repositories) — first
//! against an empty registry (everything `new`), then against one
//! built from the first scan's keys the way the command layer will
//! build it from the store — and the second pass classifies every
//! repository `known`, none `new` (PRJ-006: scanning again creates
//! no duplicates), with the harvested keys pinned distinct,
//! name-bearing and separator-clean so a degenerate key function
//! cannot pass the pair. Each registry set gives its class; the
//! priority pins are separate tests (active wins over both others;
//! an archived project still present in the exclusions stays
//! `archived` — the PRJ-017 coexistence). Alias spellings (doubled
//! separator, trailing slash; NFC/NFD under a folding rule — the
//! cross-module premise re-stated where the classifier consumes
//! it) collapse to one key and one class; classification touches no
//! filesystem (a missing path classifies by its text key); a scan
//! rooted at a SYMLINK against the canonical registry classifies
//! `new` — the documented failure mode that makes the caller's
//! canonical-root obligation real; duplicates inside a set are
//! harmless; every [`SkipReason`] passes through with its OS code
//! intact and its path readable through the envelope's `path()`;
//! and a plain directory passes through unclassified.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::paths;
use crate::scan::{ScanEvent, SkipReason};

/// The class of one found repository (ALG-053's four classes).
///
/// The words are the spec's own (`new`, `known` (active),
/// `archived`, `excluded`) and [`Display`](fmt::Display) spells them
/// exactly so, lower case — the log and dialog vocabulary. The
/// wire vocabulary of the progress event (§7.10) is the command
/// layer's contract to pin; until it does, `Display` claims only
/// the human-readable spelling, not serialization stability.
///
/// The classes are FACTS about the registry lookup, not actions:
/// what the UI does with them (checkboxes for `new`, the restore
/// offer for `archived`, the reason text for `excluded`) is
/// PRJ-002's mapping in the command layer.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ScanClass {
    /// The key is in no registry set: a repository the app has
    /// never added and never excluded — PRJ-002's "New" group,
    /// checkbox checked by default.
    New,
    /// The key is an ACTIVE project: PRJ-002's "Already added"
    /// group. The spec's word is `known`; the parenthetical
    /// `(active)` is the meaning — an archived row is NOT this
    /// class.
    Known,
    /// The key is an ARCHIVED project. The scan window shows it in
    /// "Skipped" with the reason "archived or in exclusions"
    /// (PRJ-002); DB-015's restore offer is the add flow's.
    Archived,
    /// The key is in `project_exclusions` with NO project row
    /// (`removed_scanned` or `manual`): the user told the scan to
    /// stop offering this path. Also "Skipped" with the same
    /// reason text — distinct from the walk's
    /// [`SkipReason::Excluded`], which is a folder-name fact.
    Excluded,
}

impl fmt::Display for ScanClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = match self {
            ScanClass::New => "new",
            ScanClass::Known => "known",
            ScanClass::Archived => "archived",
            ScanClass::Excluded => "excluded",
        };
        write!(f, "{word}")
    }
}

/// The registry snapshot one scan classifies against: the
/// `path_key`s of the active projects, the archived projects, and
/// the `project_exclusions` rows — ALG-053's three "known" classes,
/// leaving `new` as the fall-through.
///
/// Built by the CALLER per scan (this crate never reads the store,
/// D-093's boundary): the command layer loads the three key lists
/// and hands them over. Keys enter as they are stored — the very
/// strings the add flow computed with [`paths::path_key`] (the
/// D-086 format contract; a different fold flavor here would
/// silently split project identities). Duplicates inside one list
/// are harmless
/// (set semantics), and a key in several lists resolves by the
/// module's fixed priority, never an error — see the module docs
/// for why `archived ∩ excluded` is the normal post-archive state.
///
/// [`Default`] is the empty registry: every repository classifies
/// `new`, the first-ever scan of a folder.
///
/// # Examples
///
/// ```
/// use eunomia_fs::scan_classify::{classify_key, ScanClass, ScanRegistry};
///
/// // Archiving a scan-found project keeps the archived row AND
/// // writes the exclusion (PRJ-017) — the overlap resolves to
/// // `archived`, not an error:
/// let key = "/srv/repos/eunomia".to_string();
/// let registry = ScanRegistry::new(
///     &[],
///     std::slice::from_ref(&key),
///     std::slice::from_ref(&key),
/// );
/// assert_eq!(classify_key(&key, &registry), ScanClass::Archived);
///
/// // A key no set knows is new:
/// assert_eq!(
///     classify_key("/srv/repos/other", &registry),
///     ScanClass::New
/// );
/// ```
#[derive(Debug, Clone, Default)]
pub struct ScanRegistry {
    /// `path_key`s of projects with `state = 'active'` (DB-015).
    active: HashSet<String>,
    /// `path_key`s of projects with `state = 'archived'`.
    archived: HashSet<String>,
    /// `path_key`s of `project_exclusions` rows (§6.2).
    excluded: HashSet<String>,
}

impl ScanRegistry {
    /// Collects the three membership sets from the caller's key
    /// lists (the strings exactly as stored, see the type docs).
    ///
    /// No validation happens here and none is needed: duplicates
    /// collapse (set semantics), and keys present in more than one
    /// list classify by the fixed priority documented on the module
    /// and on [`classify_key`] — the overlap is data, not an error.
    pub fn new(active: &[String], archived: &[String], excluded: &[String]) -> Self {
        Self {
            active: active.iter().cloned().collect(),
            archived: archived.iter().cloned().collect(),
            excluded: excluded.iter().cloned().collect(),
        }
    }
}

/// A found repository with its ALG-053 facts: the path as the scan
/// reported it, its `path_key`, and the class the registry gave it.
///
/// Owned on purpose — the repositories are the payload the command
/// layer accumulates for PRJ-002's result list ("New" checkboxes,
/// "Add selected"), so the data must outlive the scan callback that
/// produced it. The path is stored VERBATIM (the D-086 ruling: NFC
/// and folding belong to the key only); the key is the identity the
/// registry matched on, so the class and the key can never disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassifiedRepository {
    /// The repository root exactly as the walk reported it.
    path: PathBuf,
    /// The [`paths::path_key`] of `path` — the registered identity.
    path_key: String,
    /// What the registry said about that key.
    class: ScanClass,
}

impl ClassifiedRepository {
    /// The repository root exactly as the walk reported it —
    /// verbatim. It is canonical only when the caller canonicalized
    /// the scan root before `scan_repositories` (the module docs'
    /// caller obligation); a symlinked root yields link-spelled
    /// paths.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The identity key the registry was matched against — the same
    /// string the store holds when the class is not
    /// [`ScanClass::New`].
    #[must_use]
    pub fn path_key(&self) -> &str {
        &self.path_key
    }

    /// The registry's verdict for [`Self::path_key`].
    #[must_use]
    pub fn class(&self) -> ScanClass {
        self.class
    }
}

/// One [`ScanEvent`], classified: the shape ready to serialize into
/// the scan's progress event (SYS-003's `scan://progress`).
///
/// The mapping from [`ScanEvent`] is 1:1 and total — every walk
/// event becomes exactly one classified event, so the command layer
/// writes ONE match over this type and cannot drop a fact:
/// repositories carry their [`ClassifiedRepository`]; plain folders
/// (the "folders checked" counter) and skips (the "Skipped" group
/// with its reason) pass through BY REFERENCE — no allocation for
/// the events that need no new facts, which is most of them. The
/// borrow ties the value to the callback's `&ScanEvent`, matching
/// how the command layer consumes it: classify, serialize, done.
#[derive(Clone, Debug)]
pub enum ClassifiedScanEvent<'a> {
    /// A found repository, keyed and classified (ALG-053). The one
    /// variant carrying new facts, owned (see
    /// [`ClassifiedRepository`]).
    Repository(ClassifiedRepository),
    /// A plain folder, passed through: the "folders checked"
    /// counter of PRJ-002. Classification has nothing to add.
    Directory {
        /// The folder's path.
        path: &'a Path,
    },
    /// A folder the walk refused or skipped, passed through with
    /// the reason verbatim (PRJ-002's "Skipped" group): the walker's
    /// vocabulary is already the UI's, and the OS code rides the
    /// [`SkipReason`] untouched.
    Skipped {
        /// The skipped entry's path.
        path: &'a Path,
        /// Why it was skipped, by reference into the source event.
        reason: &'a SkipReason,
    },
}

impl ClassifiedScanEvent<'_> {
    /// The path this event is about — the classified repository's
    /// root, the checked folder, or the skipped entry (the mirror of
    /// [`ScanEvent::path`], so consumers need not re-match for the
    /// one field every variant carries).
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            ClassifiedScanEvent::Repository(classified) => classified.path(),
            ClassifiedScanEvent::Directory { path } | ClassifiedScanEvent::Skipped { path, .. } => {
                path
            }
        }
    }
}

/// The class of one `path_key` under a registry: the whole of
/// ALG-053's lookup, pure and total.
///
/// The fixed priority for a key in several sets (the module docs
/// have the full grounding): active beats everything (DB-015's one
/// row per key; D-10's restore clears the exclusion), an archived
/// row beats its exclusion side effect (PRJ-017's coexistence), a
/// bare exclusion answers only when no project row exists, and a
/// key no set knows is new.
///
/// # Examples
///
/// ```
/// use eunomia_fs::scan_classify::{classify_key, ScanClass, ScanRegistry};
///
/// let key = "/srv/repos/eunomia".to_string();
/// let registry = ScanRegistry::new(std::slice::from_ref(&key), &[], &[]);
///
/// // Active wins even over a leftover exclusion:
/// let conflicted =
///     ScanRegistry::new(std::slice::from_ref(&key), &[], std::slice::from_ref(&key));
/// assert_eq!(classify_key(&key, &conflicted), ScanClass::Known);
/// ```
#[must_use = "the class is the answer; computing and dropping it was wasted lookup"]
pub fn classify_key(path_key: &str, registry: &ScanRegistry) -> ScanClass {
    if registry.active.contains(path_key) {
        ScanClass::Known
    } else if registry.archived.contains(path_key) {
        ScanClass::Archived
    } else if registry.excluded.contains(path_key) {
        ScanClass::Excluded
    } else {
        ScanClass::New
    }
}

/// Keys and classifies one found repository (ALG-053's "for a found
/// path, the `path_key` and class are computed").
///
/// The key comes from [`paths::path_key`] applied to `path` AS
/// GIVEN — no canonicalization, no filesystem access. The identity
/// that key promises holds when `path` is the walker's output under
/// a canonicalized scan root — the CALLER's obligation, not the
/// walker's guarantee (see the module docs: the walker accepts a
/// symlinked root and reports its link-spelled paths). The class is
/// [`classify_key`] on that key. Total: any path classifies; a path
/// that does not exist classifies by its text key.
///
/// # Examples
///
/// ```
/// use eunomia_fs::paths::path_key;
/// use eunomia_fs::scan_classify::{classify_repository, ScanClass, ScanRegistry};
/// use std::path::Path;
///
/// let repo = Path::new("/srv/repos/eunomia");
///
/// // The first scan of a folder: the registry is empty, everything
/// // is new.
/// let empty = ScanRegistry::default();
/// assert_eq!(classify_repository(repo, &empty).class(), ScanClass::New);
///
/// // After the user added it, the same path is known — the
/// // acceptance criterion, one lookup wide:
/// let key = path_key(repo);
/// let registry = ScanRegistry::new(&[key], &[], &[]);
/// assert_eq!(classify_repository(repo, &registry).class(), ScanClass::Known);
/// ```
#[must_use = "the classification allocates the key; dropping it wastes the work"]
pub fn classify_repository(path: &Path, registry: &ScanRegistry) -> ClassifiedRepository {
    let path_key = paths::path_key(path);
    let class = classify_key(&path_key, registry);
    ClassifiedRepository {
        path: path.to_path_buf(),
        path_key,
        class,
    }
}

/// Classifies one walk event into the serializable
/// [`ClassifiedScanEvent`]: repositories gain their key and class,
/// everything else passes through by reference.
///
/// The command layer calls this inside its
/// [`scan_repositories`](crate::scan::scan_repositories) callback
/// and serializes the result into the throttled progress event; the
/// cancellation check and the throttle are not here (they are the
/// walk's and the command layer's, ALG-053's two other clauses).
/// Total and allocation-free except for the one repository
/// classification.
///
/// # Examples
///
/// ```
/// use eunomia_fs::scan::{ScanEvent, SkipReason};
/// use eunomia_fs::scan_classify::{classify_entry, ClassifiedScanEvent, ScanRegistry};
/// use std::path::Path;
///
/// let registry = ScanRegistry::default();
/// let event = ScanEvent::Skipped {
///     path: "/scan/worktree".into(),
///     reason: SkipReason::SubmoduleOrWorktree,
/// };
///
/// // A skip passes through with its reason — the UI vocabulary is
/// // the walker's own:
/// match classify_entry(&event, &registry) {
///     ClassifiedScanEvent::Skipped { reason, .. } => {
///         assert_eq!(reason.to_string(), "submodule or worktree");
///     }
///     other => panic!("a skip must pass through as a skip: {other:?}"),
/// }
/// ```
#[must_use = "the envelope is the progress event's payload; dropping it loses the classification"]
pub fn classify_entry<'a>(
    event: &'a ScanEvent,
    registry: &ScanRegistry,
) -> ClassifiedScanEvent<'a> {
    match event {
        ScanEvent::Repository { path } => {
            ClassifiedScanEvent::Repository(classify_repository(path, registry))
        }
        ScanEvent::Directory { path } => ClassifiedScanEvent::Directory { path },
        ScanEvent::Skipped { path, reason } => ClassifiedScanEvent::Skipped { path, reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{CancelToken, ScanOptions, scan_repositories};
    use std::io;
    use std::num::NonZeroUsize;

    use tempfile::TempDir;

    /// Creates `dir/name/.git` as a directory (ALG-052's repository
    /// form) and returns the repository's path.
    fn repo_with_git_dir(dir: &Path, name: &str) -> PathBuf {
        let repo = dir.join(name);
        std::fs::create_dir_all(repo.join(".git"))
            .expect("creating the fixture repository must work");
        repo
    }

    /// Runs one scan of `root` classified against `registry` — the
    /// command layer's flow (T-076) in miniature: walk, classify
    /// every repository event, keep the classified records.
    fn scan_classified(root: &Path, registry: &ScanRegistry) -> Vec<ClassifiedRepository> {
        let options = ScanOptions {
            max_depth: NonZeroUsize::new(6).expect("6 is non-zero"),
            exclude_names: Vec::new(),
        };
        let cancel = CancelToken::new();
        let found = std::sync::Mutex::new(Vec::new());
        let outcome = scan_repositories(root, &options, &cancel, |event| {
            if let ScanEvent::Repository { path } = event {
                found
                    .lock()
                    .expect("the test collector cannot be poisoned")
                    .push(classify_repository(path, registry));
            }
        });
        assert_eq!(outcome, crate::scan::ScanOutcome::Completed);
        found
            .into_inner()
            .expect("the test collector cannot be poisoned")
    }

    #[test]
    fn a_rescan_of_the_same_folder_classifies_known_not_new() {
        // T-075's acceptance criterion, end to end against a real
        // tree: the first scan of a folder against an empty registry
        // finds only new repositories; after "the user added them"
        // (their keys become the active set — exactly what T-076
        // will load from the store), the SECOND scan of the same
        // folder classifies every repository known, none new
        // (PRJ-006: scanning again creates no duplicates). The tree
        // carries a NESTED repository (ALG-051) and the keys are
        // pinned distinct, name-bearing and separator-clean, so a
        // degenerate key function (a constant key, or one that
        // collapsed the nested pair) cannot pass the pair.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let alpha = repo_with_git_dir(dir.path(), "alpha");
        repo_with_git_dir(&alpha, "inner");
        repo_with_git_dir(dir.path(), "beta");

        let first = scan_classified(dir.path(), &ScanRegistry::default());
        assert_eq!(
            first.len(),
            3,
            "the outer, nested and sibling repositories must all be found; got: {first:?}"
        );
        assert!(
            first.iter().all(|r| r.class() == ScanClass::New),
            "against an empty registry every repository is new; got: {first:?}"
        );

        let keys: Vec<String> = first
            .iter()
            .map(ClassifiedRepository::path_key)
            .map(str::to_string)
            .collect();
        assert_eq!(
            keys.iter().collect::<std::collections::HashSet<_>>().len(),
            3,
            "three distinct repositories must keep three distinct keys; got: {keys:?}"
        );
        for name in ["alpha", "beta", "inner"] {
            assert!(
                keys.iter().any(|key| key.contains(name)),
                "the {name} repository's key must carry its name; got: {keys:?}"
            );
        }
        assert!(
            keys.iter().all(|key| !key.ends_with('/')),
            "keys carry no trailing separator; got: {keys:?}"
        );

        let registry = ScanRegistry::new(&keys, &[], &[]);

        let second = scan_classified(dir.path(), &registry);
        assert_eq!(
            second.len(),
            3,
            "the rescan must find the same three; got: {second:?}"
        );
        assert!(
            second.iter().all(|r| r.class() == ScanClass::Known),
            "the rescan must classify every repository known, not new; got: {second:?}"
        );
        let alpha = second
            .iter()
            .find(|r| r.path().ends_with("alpha"))
            .expect("alpha must be found again");
        assert_eq!(alpha.class(), ScanClass::Known);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_scan_root_classifies_new_against_the_canonical_registry() {
        // The documented failure mode behind the caller's
        // canonical-root obligation (see the module docs): the walker
        // accepts a symlinked root and reports link-SPELLED paths,
        // so the very repository the registry knows by its canonical
        // key classifies new. Pinned so the obligation cannot be
        // assumed away — the command layer must canonicalize the
        // root before scan_repositories.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_dir(dir.path(), "repo");
        let link = dir.path().join("link-root");
        std::os::unix::fs::symlink(&repo, &link).expect("creating the root symlink must work");

        // The registry holds the CANONICAL key — what the add flow
        // stores (D-086: canonicalize first, then key).
        let canonical = paths::canonicalize(&repo).expect("the fixture repository must resolve");
        let canonical_key = paths::path_key(&canonical);
        let registry = ScanRegistry::new(std::slice::from_ref(&canonical_key), &[], &[]);

        let found = scan_classified(&link, &registry);
        assert_eq!(
            found.len(),
            1,
            "the symlinked root is the repository; got: {found:?}"
        );
        assert_eq!(
            found[0].class(),
            ScanClass::New,
            "the link-spelled path keys by its text and misses the canonical key — \
             this is why the caller must canonicalize the root; got: {found:?}"
        );
        assert_eq!(found[0].path_key(), paths::path_key(&link));
        assert_ne!(found[0].path_key(), paths::path_key(&canonical));
    }

    #[test]
    fn display_spells_the_spec_words() {
        // Display is the log and dialog vocabulary: the spec's own
        // four words, lower case (ALG-053) — pinned so a "friendly"
        // rewording cannot silently change what the logs show.
        assert_eq!(ScanClass::New.to_string(), "new");
        assert_eq!(ScanClass::Known.to_string(), "known");
        assert_eq!(ScanClass::Archived.to_string(), "archived");
        assert_eq!(ScanClass::Excluded.to_string(), "excluded");
    }

    #[test]
    fn each_registry_set_gives_its_class() {
        // The three non-new classes are set membership on one key.
        // The key itself is pinned spelling-independently first
        // (name-bearing, separator-clean), so the equality below can
        // fail for the right reason — a degenerate key would be
        // caught here, not laundered through the same call.
        let path = Path::new("/srv/repos/eunomia");
        let key = paths::path_key(path);
        assert!(
            key.contains("repos/eunomia"),
            "the key carries the name; got: {key}"
        );
        assert!(
            !key.ends_with('/'),
            "the key has no trailing separator; got: {key}"
        );
        for (registry, expected) in [
            (
                ScanRegistry::new(std::slice::from_ref(&key), &[], &[]),
                ScanClass::Known,
            ),
            (
                ScanRegistry::new(&[], std::slice::from_ref(&key), &[]),
                ScanClass::Archived,
            ),
            (
                ScanRegistry::new(&[], &[], std::slice::from_ref(&key)),
                ScanClass::Excluded,
            ),
        ] {
            let classified = classify_repository(path, &registry);
            assert_eq!(classified.class(), expected);
            assert_eq!(
                classified.path_key(),
                key,
                "the classified key is the registered one, unmangled ({expected})"
            );
            assert_eq!(classified.path(), path);
        }
    }

    #[test]
    fn the_active_set_wins_over_the_other_two() {
        // Priority clause 1: an active project is known whatever
        // stale overlaps the snapshot carries. active+archived
        // cannot be two rows (DB-015) but the snapshot is caller
        // data and the answer stays deterministic. Duplicates
        // inside a list ride along harmlessly.
        let key = "/srv/repos/eunomia".to_string();
        let with_duplicates = vec![key.clone(), key.clone()];
        for registry in [
            ScanRegistry::new(
                std::slice::from_ref(&key),
                std::slice::from_ref(&key),
                std::slice::from_ref(&key),
            ),
            ScanRegistry::new(&with_duplicates, &[], std::slice::from_ref(&key)),
            ScanRegistry::new(std::slice::from_ref(&key), std::slice::from_ref(&key), &[]),
        ] {
            assert_eq!(
                classify_key("/srv/repos/eunomia", &registry),
                ScanClass::Known
            );
        }
    }

    #[test]
    fn an_archived_project_still_listed_in_exclusions_is_archived() {
        // Priority clause 2, the load-bearing one: archiving a
        // scan-found project KEEPS the archived row and WRITES an
        // exclusion for the same key (PRJ-017, §6.4; D-093 pins the
        // coexistence) — that steady state must classify archived,
        // with the restore flow as the answer, not as an error.
        let key = "/srv/repos/eunomia".to_string();
        let registry =
            ScanRegistry::new(&[], std::slice::from_ref(&key), std::slice::from_ref(&key));
        assert_eq!(
            classify_key("/srv/repos/eunomia", &registry),
            ScanClass::Archived
        );
    }

    #[test]
    fn an_empty_registry_classifies_everything_new() {
        // The first-ever scan: no sets, everything new — both at
        // the key level and through a full path classification.
        let registry = ScanRegistry::default();
        assert_eq!(classify_key("/never/seen", &registry), ScanClass::New);
        assert_eq!(
            classify_repository(Path::new("/never/seen"), &registry).class(),
            ScanClass::New
        );
    }

    #[test]
    fn alias_spellings_of_one_path_get_one_class() {
        // The key collapses separator runs and trailing separators
        // on every platform rule (DB-030), and classification
        // inherits the collapse: two spellings, one key, one class.
        // The scan itself yields clean joins; this pins that the
        // classifier adds no spelling sensitivity of its own.
        let clean = Path::new("/srv/repos/eunomia");
        let messy = Path::new("/srv//repos/eunomia/");
        let registry = ScanRegistry::new(&[paths::path_key(clean)], &[], &[]);

        let by_clean = classify_repository(clean, &registry);
        let by_messy = classify_repository(messy, &registry);

        assert_eq!(by_clean.class(), ScanClass::Known);
        assert_eq!(by_messy.class(), ScanClass::Known);
        assert_eq!(
            by_messy.path_key(),
            by_clean.path_key(),
            "the two spellings must key identically"
        );
    }

    #[test]
    fn folded_spellings_share_one_class_under_folding_rules() {
        // A doc-pin of the cross-module claim where the classifier
        // consumes it: the folding rules (Windows/macOS, DB-030 —
        // the rule's own battery lives in `paths.rs`) collapse NFC
        // and NFD spellings of one name into one key, and ONE KEY
        // MEANS ONE CLASS through this module's lookup. Run at the
        // key level with the rule fed explicitly so the property is
        // exercised on every OS; the classifier's own leg is the
        // platform rule.
        let nfc = paths::path_key_with_rule("/srv/caf\u{e9}", paths::PathKeyRule::MacOs);
        let nfd = paths::path_key_with_rule("/srv/cafe\u{301}", paths::PathKeyRule::MacOs);
        assert_eq!(
            nfc, nfd,
            "the premise, pinned in paths.rs: the folding rule collapses the spellings"
        );

        let registry = ScanRegistry::new(&[], &[], &[nfd]);
        assert_eq!(classify_key(&nfc, &registry), ScanClass::Excluded);
    }

    #[test]
    fn classification_touches_no_filesystem() {
        // Purity pinned behaviorally: a path that does not exist
        // still classifies, by the key of its text (a canonicalizing
        // classifier would fail NotFound here). The classifier never
        // opens anything — ALG-054's posture, one layer up.
        let missing = Path::new("/definitely/not/here/repo");
        let key = paths::path_key(missing);
        let registry = ScanRegistry::new(&[key], &[], &[]);

        assert_eq!(
            classify_repository(missing, &registry).class(),
            ScanClass::Known
        );
    }

    #[test]
    fn classify_entry_passes_every_skip_reason_through() {
        // The walker's whole skip vocabulary rides the envelope
        // unchanged — reason by reference, OS code intact
        // (PRJ-007), path exact.
        let registry = ScanRegistry::default();
        let cases: [(&str, SkipReason, Option<i32>); 3] = [
            ("/scan/node_modules", SkipReason::Excluded, None),
            ("/scan/worktree", SkipReason::SubmoduleOrWorktree, None),
            (
                "/scan/locked",
                SkipReason::Access {
                    source: io::Error::from_raw_os_error(13),
                },
                Some(13),
            ),
        ];
        for (path, reason, os_code) in cases {
            // The walker's own verdict kind, taken before the event
            // takes ownership of the reason.
            let expected_reason = std::mem::discriminant(&reason);
            let event = ScanEvent::Skipped {
                path: PathBuf::from(path),
                reason,
            };
            let classified = classify_entry(&event, &registry);
            let ClassifiedScanEvent::Skipped {
                path: passed_path,
                reason: passed_reason,
            } = &classified
            else {
                panic!("a skip must pass through as a skip: {path}");
            };
            assert_eq!(*passed_path, Path::new(path));
            assert_eq!(passed_reason.os_code(), os_code);
            assert_eq!(
                std::mem::discriminant(*passed_reason),
                expected_reason,
                "the passed-through reason must keep the walker's verdict kind: {path}"
            );
            // The accessor every variant shares answers for skips
            // too — the arm a "Skipped"-group consumer would rely on.
            assert_eq!(classified.path(), Path::new(path));
        }
    }

    #[test]
    fn classify_entry_classifies_repositories_and_passes_plain_folders() {
        // The envelope's 1:1 mapping: a repository event gains its
        // key and class (the key equal to the registered one), a
        // plain folder passes through untouched.
        let key = paths::path_key(Path::new("/srv/repos/eunomia"));
        let registry = ScanRegistry::new(&[key], &[], &[]);

        let repository_event = ScanEvent::Repository {
            path: PathBuf::from("/srv/repos/eunomia"),
        };
        let ClassifiedScanEvent::Repository(classified) =
            classify_entry(&repository_event, &registry)
        else {
            panic!("a repository event must classify as a repository");
        };
        assert_eq!(classified.class(), ScanClass::Known);
        assert_eq!(classified.path(), Path::new("/srv/repos/eunomia"));

        let directory_event = ScanEvent::Directory {
            path: PathBuf::from("/srv/plain"),
        };
        let ClassifiedScanEvent::Directory { path } = classify_entry(&directory_event, &registry)
        else {
            panic!("a plain folder must pass through as a directory");
        };
        assert_eq!(path, Path::new("/srv/plain"));

        // The envelope's own path() mirrors ScanEvent::path for all
        // three variants.
        assert_eq!(
            classify_entry(&repository_event, &registry).path(),
            Path::new("/srv/repos/eunomia")
        );
        assert_eq!(
            classify_entry(&directory_event, &registry).path(),
            Path::new("/srv/plain")
        );
    }
}
