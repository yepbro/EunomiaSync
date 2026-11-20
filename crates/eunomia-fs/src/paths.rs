//! Path normalization and validation.
//!
//! Planned responsibility (docs/spec.md 5.2): canonicalize and normalize
//! repository paths so that they compare equal across platforms and app
//! restarts — `dunce` keeps canonical Windows paths free of verbatim
//! (`\\?\`) prefixes — and provide the path checks used when validating
//! paths that arrive from the UI (spec 5.6, SEC-003: every UI-supplied
//! path is checked in Rust). The registered-project and session-picked
//! allowlists that feed those checks live in the app and database layers;
//! this module supplies the normalization primitives and checks, not the
//! allowlist policy.
//!
//! Implemented so far: the project-folder availability check
//! ([`check_project_available`], spec 6.9's project-path row, 9.4
//! preflight step 1) and the project-path canonicalization with the
//! `path_key` identity key (DB-030, DB-031, D-07). Still planned: the
//! SEC-003 allowlist path checks.
//!
//! # Canonicalization and the path_key (DB-030, DB-031, D-07)
//!
//! A project path enters the app once and is identified from then on
//! by its KEY, so DB-030's two halves are pinned here separately.
//! [`canonicalize`] is `dunce::canonicalize`: the real path with
//! symlinks resolved, without the Windows verbatim (`\\?\`)
//! extended-length prefix WHERE DUNCE CAN SAFELY STRIP IT — plain
//! drive paths always strip; verbatim UNC shares, paths beyond the
//! 260-character limit and reserved DOS component names keep the
//! `\\?\` spelling, because stripping those would produce paths
//! Win32 cannot open (dunce 1.0.5's own safety rule; DB-032 network
//! drives and the long-path posture land there deliberately). Its
//! output is the STORED path, verbatim — NFC and folding apply to the
//! key only, never to the stored path (owner ruling, 2026-11-19).
//! [`path_key`] builds the identity key
//! of an already-canonicalized path — pure and total: separators
//! become `/`, interior and trailing separator runs collapse, and the
//! per-OS Unicode rule applies. On Windows and macOS the key is
//! NFC-normalized and case-folded with SIMPLE folding — the 1:1
//! mappings (statuses C and S) of Unicode CaseFolding.txt, the
//! filesystem-faithful rule: ligatures have no simple fold and stay
//! distinct (`ﬁ` ≠ `fi`), while 1:1 mappings do fold (`ſ` → `s`,
//! ẞ → ß, KELVIN SIGN → k) — the filesystem-faithful rule under
//! which names NTFS `$UpCase` and APFS treat as distinct keep
//! distinct keys. The pipeline is NFC → simple fold →
//! NFC: case folding does not preserve normalization formats
//! (CaseFolding.txt's own header note), so the text is normalized
//! again after folding, and a decomposed spelling of one name folds
//! to the same key as its precomposed spelling. On Linux the key
//! keeps case and bytes (DB-030's last clause): two byte-different
//! names are two different folders there, and get different keys.
//! The rule itself lives in [`PathKeyRule`]:
//! [`path_key_with_rule`] is the pure rule-parameterized builder
//! (unit-testable on every OS by feeding the rule explicitly),
//! [`path_key`] selects the running platform's rule.
//!
//! Backslash gets its own sentence: it is a separator on Windows and
//! a legal NAME byte on Unix, so the separator rewrite is
//! rule-scoped — the Windows rule splits on both `/` and `\`, the
//! Linux and macOS rules on `/` only, and a Unix folder named `a\b`
//! never collides with a sibling `a/b`.
//!
//! Unix paths are bytes; the key is TEXT (a SQLite TEXT column,
//! §6.2). Non-UTF-8 path bytes reach the key as U+FFFD replacement
//! characters (`Path::to_string_lossy`) — the same lossy form the
//! app's logs and dialogs show for such paths. The known cost,
//! documented here and in DECISIONS.md D-086: on a byte-preserving
//! filesystem, two names differing only in invalid-byte sequences
//! collapse to one key (the second add hits the UNIQUE column) —
//! whether to reject non-UTF-8 paths at the add/scan seam instead is
//! a format-contract decision that stays with the owner. And the
//! rule set above is a FORMAT CONTRACT otherwise too: keys are
//! stored in SQLite and compared across app versions, so changing
//! the fold flavor, the normalization, or the separator shape later
//! would silently split or merge project identities (the
//! DECISIONS.md D-082 posture for snapshot hashes).
//!
//! # Testing plan
//!
//! Availability is tested through the public function against real
//! temporary fixtures: the happy `.git`-directory and `.git`-file
//! forms, a plain file and a missing path rejected, a vanished `.git`
//! flipping the verdict, permission-denied folders (skipped loudly on
//! a root account, where mode bits are not enforced), the OS error
//! code riding on the typed error, and the DB-032 contract that a
//! folder on another filesystem (and a path shaped like a
//! scan-excluded name) passes like any local folder. Canonicalization
//! is tested against real fixtures for the dotted and symlinked
//! forms, the typed failure, and the verbatim stored bytes, and the
//! key rules are tested purely — the Linux, Windows and macOS
//! behaviors all run on every OS by feeding [`PathKeyRule`]
//! explicitly, with per-OS literal pins on the platform wrappers
//! riding the CI matrix (including Windows-leg tests of the verbatim
//! prefix and the DB-031 symlink collapse), the NFC-vs-NFD collapse,
//! the simple-fold fidelity battery (ﬁ/ſ/ß/ẞ/İ/KELVIN SIGN),
//! separator shapes including the Unix backslash and the verbatim-UNC
//! key spelling, and the DB-031 one-folder-one-key symlink acceptance
//! test. The SEC-003 checks keep their planned coverage: rejection of
//! paths outside the registered or session-allowed set.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use unicode_case_mapping::case_folded;
use unicode_normalization::UnicodeNormalization;

/// Checks that a registered project's folder is available: it exists,
/// is a directory, is readable, and contains a `.git` entry.
///
/// Spec 6.9's project-path row ("Exists, is a folder with `.git`
/// (folder or file), is readable") and 9.4 preflight step 1 ("Check
/// that the folder is available. No access: `skipped`,
/// `E_PROJECT_UNAVAILABLE`"): the startup pass, the window-open pass
/// and the sync pipeline (DB-032) all reduce to this one verdict, and
/// D-09's "unavailable" project is the failure half of it.
///
/// # The verdict is four probes, nothing else
///
/// 1. `stat` of the root (symlinks followed: a project root that is
///    itself a link to the real repository checks the repository).
///    `NotFound` → [`ProjectUnavailableError::Missing`]; any other OS
///    refusal — denied access to the folder or an ancestor, a
///    disconnected drive's EIO/ENODEV, a symlink loop, a name too
///    long — → [`ProjectUnavailableError::NotReadable`]: the folder is
///    not plainly gone, it cannot be inspected, and the carried
///    [`io::Error`] keeps the exact OS code.
/// 2. `is_dir()` — anything else on the path (a plain file, or a
///    symlink to one) is [`ProjectUnavailableError::NotADirectory`].
/// 3. One probe read: `read_dir` opens the directory, which is what
///    "is readable" means for a directory (listability). Permission
///    errors surface here with their OS code. A search-only folder
///    (Unix `x` without `r`) is therefore reported unavailable even
///    though named paths inside could still be opened — the stricter
///    of the two readings, taken because the app itself lists the
///    folder (the repository scan reads entry names, ALG-054) rather
///    than poking fixed paths.
/// 4. `symlink_metadata(<root>/.git)` — ANY entry named `.git` counts:
///    a directory (a normal repository) or a file (a worktree or
///    submodule gitdir pointer — ALG-052's other form). The scan
///    distinguishes the two (ALG-052 skips the file form); availability
///    only needs "there is a `.git` entry", so the file form is
///    deliberately not rejected. `symlink_metadata` rather than
///    `metadata`: presence, not validity, is the contract — a `.git`
///    symlink is still an entry, and whether it points at a healthy
///    gitdir is git's business, not this check's. `NotFound` here is
///    [`ProjectUnavailableError::NoGitEntry`]; any other error is
///    [`ProjectUnavailableError::NotReadable`].
///
/// # Network and removable drives are allowed (DB-032)
///
/// The check is purely metadata: one `stat`, one directory open, one
/// entry `stat`. It reads no device numbers, no mount tables, no
/// volume flags, and sniffs no path shapes (`/mnt/…`, `\\UNC\…`) —
/// a repository on an NFS share, a USB drive, a `\\server\share` UNC
/// path or a tmpfs mount passes exactly like a local one. There is
/// deliberately no seam for a drive-type heuristic to plug into: the
/// four probes above are the entire decision. The tests pin the
/// contract with a cross-filesystem fixture and with paths shaped
/// like names the repository scan excludes.
///
/// # Races are honest (TOCTOU)
///
/// The probes are separate OS calls over a live filesystem: the folder
/// can lose its `.git` between them, a drive can drop mid-check, a
/// plain file can replace the folder between `stat` and the probe
/// read (that surfaces as [`ProjectUnavailableError::NotReadable`]
/// with the OS code for "not a directory"). The function reports what
/// the probes observed at their moment — no snapshot is taken and no
/// atomicity is pretended. The availability verdict is recomputed at
/// every startup, window open and sync (DB-032), so a flip between two
/// checks is simply the next check's verdict.
///
/// # Errors
///
/// `Ok(())` means the folder is available. Every failure maps, at the
/// command layer, to `E_PROJECT_UNAVAILABLE` (§10: folder moved or
/// deleted, drive disconnected, no read rights) with the OS code as
/// detail — [`ProjectUnavailableError::io_error`] and
/// [`ProjectUnavailableError::os_code`] feed that detail. The reasons
/// stay distinguishable in the enum because the add-project flow's
/// `E_NOT_A_REPO` (a folder without `.git`) is a different consumer
/// that may reuse the same check.
pub fn check_project_available(root: &Path) -> Result<(), ProjectUnavailableError> {
    let metadata = fs::metadata(root).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ProjectUnavailableError::Missing { source }
        } else {
            ProjectUnavailableError::NotReadable { source }
        }
    })?;
    if !metadata.is_dir() {
        return Err(ProjectUnavailableError::NotADirectory);
    }
    // The open is the probe read: `read_dir` opens the directory
    // eagerly, so a permission or I/O failure surfaces on this call;
    // the entries themselves are not needed.
    fs::read_dir(root).map_err(|source| ProjectUnavailableError::NotReadable { source })?;
    match fs::symlink_metadata(root.join(".git")) {
        Ok(_) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            Err(ProjectUnavailableError::NoGitEntry)
        }
        Err(source) => Err(ProjectUnavailableError::NotReadable { source }),
    }
}

/// Why [`check_project_available`] said no.
///
/// One variant per §10 situation behind `E_PROJECT_UNAVAILABLE` (plus
/// the plain "not a folder" and "no `.git` entry" outcomes of the §6.9
/// row), so the command layer can log the reason and the OS code
/// without re-deriving them from strings. I/O-backed variants carry
/// the [`io::Error`] itself as their `source` — its `raw_os_error`
/// and `ErrorKind` are §10's first-class log facts, and
/// `ErrorDetails::from_io_error` in the app layer consumes exactly
/// that shape.
#[derive(Debug)]
pub enum ProjectUnavailableError {
    /// The folder is not there: `stat` answered ENOENT — moved,
    /// deleted, or the drive unmounted (D-09's situation list).
    Missing {
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The path exists but is not a directory: a plain file (or a
    /// symlink to one) occupies the registered path.
    NotADirectory,
    /// The folder cannot be inspected or read: access denied on the
    /// folder or an ancestor, the drive disconnected mid-check, a
    /// symlink loop — every OS refusal that is not a plain
    /// `NotFound`. The exact OS code rides in the source error.
    NotReadable {
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The folder is readable but has no `.git` entry: the directory
    /// lost its repository, or never had one.
    NoGitEntry,
}

impl ProjectUnavailableError {
    /// The I/O error behind this failure, when an OS call produced
    /// it; `None` for the verdicts that are plain facts about the
    /// path (`NotADirectory`, `NoGitEntry`) rather than OS failures.
    pub fn io_error(&self) -> Option<&io::Error> {
        match self {
            ProjectUnavailableError::Missing { source }
            | ProjectUnavailableError::NotReadable { source } => Some(source),
            ProjectUnavailableError::NotADirectory | ProjectUnavailableError::NoGitEntry => None,
        }
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`): §10
    /// names it a first-class fact of every error log line. `None`
    /// where no OS call failed.
    pub fn os_code(&self) -> Option<i32> {
        self.io_error().and_then(io::Error::raw_os_error)
    }
}

impl fmt::Display for ProjectUnavailableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectUnavailableError::Missing { source } => {
                write!(f, "project folder does not exist: {source}")
            }
            ProjectUnavailableError::NotADirectory => {
                write!(f, "project path is not a directory")
            }
            ProjectUnavailableError::NotReadable { source } => {
                write!(f, "project folder cannot be read: {source}")
            }
            ProjectUnavailableError::NoGitEntry => {
                write!(f, "project folder contains no .git entry")
            }
        }
    }
}

impl std::error::Error for ProjectUnavailableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io_error()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

// ---- DB-030 / DB-031 / D-07: canonicalization and the path_key -----

/// The per-OS Unicode rule DB-030 pins for `path_key` construction
/// (visible in the API so the spec's per-OS wording has a home and a
/// future divergence point).
///
/// Windows and macOS share the folding body — NFC plus SIMPLE case
/// folding, the filesystem-faithful 1:1 rule — but stay separate
/// variants: the spec names them per-OS (the Windows variant is the
/// stand-in for the NTFS `$UpCase` comparison), and a platform that
/// later diverges changes one variant instead of the shared body. The
/// variants are also the pure builder's test surface:
/// [`path_key_with_rule`] feeds any rule on any OS.
///
/// # Examples
///
/// ```
/// use eunomia_fs::paths::{path_key_with_rule, PathKeyRule};
///
/// // Linux keeps case; Windows and macOS fold it away:
/// assert_eq!(path_key_with_rule("/Repo", PathKeyRule::Linux), "/Repo");
/// assert_eq!(path_key_with_rule("/Repo", PathKeyRule::Windows), "/repo");
/// assert_eq!(path_key_with_rule("/Repo", PathKeyRule::MacOs), "/repo");
/// ```
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum PathKeyRule {
    /// Linux: separators only — case kept, no Unicode normalization
    /// (DB-030: "on Linux, case is kept and no normalization is
    /// applied").
    Linux,
    /// Windows: NFC + simple case folding, the faithful stand-in for
    /// the NTFS `$UpCase` comparison (DB-030).
    Windows,
    /// macOS: NFC + simple case folding (DB-030).
    MacOs,
}

/// Canonicalizes a project path into the form the app stores
/// (DB-030): the real path of `path` with symlinks resolved —
/// `dunce::canonicalize`, which strips the Windows verbatim
/// (`\\?\`) prefix wherever stripping is safe (plain drive paths)
/// and keeps it where Win32 could not open the stripped spelling
/// (verbatim UNC shares, paths beyond 260 characters, reserved DOS
/// component names).
///
/// The output is the stored `projects.path` verbatim. It is NOT
/// normalized: NFC and case folding belong to the KEY only
/// ([`path_key`]), by the owner ruling recorded in DB-030 — two
/// byte-different spellings of one name are two real folders on a
/// byte-preserving filesystem, and the stored path must carry the
/// bytes the filesystem answers with. DB-031's aliasing is resolved
/// HERE, by keying only canonicalized paths: a symlink resolves to
/// its target before any key is built.
///
/// # Errors
///
/// [`CanonicalizeError`] when the OS cannot resolve the path — it
/// does not exist (a moved, deleted or unmounted folder; D-09's
/// situation), access is denied, a link loop cannot be followed, or
/// a component is not a directory. The typed error carries the input
/// path text and the OS error with its raw code, for the §10 log
/// line.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use eunomia_fs::paths::canonicalize;
///
/// // A missing path is a typed error carrying the OS code, never a
/// // panic:
/// let error = canonicalize(Path::new("/definitely/not/here")).unwrap_err();
/// assert!(error.os_code().is_some());
/// ```
pub fn canonicalize(path: &Path) -> Result<std::path::PathBuf, CanonicalizeError> {
    dunce::canonicalize(path).map_err(|source| CanonicalizeError {
        path: path.to_string_lossy().into_owned(),
        source,
    })
}

/// The identity key of a project path (DB-030, DB-031): canonicalizes
/// first, then keys — the composition the add-project flow will call
/// (the flows land in a later task), and the seam DB-031's symlink
/// rule lives in. A folder and a symlink to
/// it resolve to the same canonical form, so they get ONE key.
///
/// # Errors
///
/// [`CanonicalizeError`], as in [`canonicalize`].
///
/// # Examples
///
/// ```
/// use eunomia_fs::paths::{canonical_path_key, canonicalize, path_key};
///
/// let dir = tempfile::tempdir().unwrap();
/// let repo = dir.path().join("repo");
/// std::fs::create_dir(&repo).unwrap();
///
/// let canonical = canonicalize(&repo).unwrap();
/// let key = canonical_path_key(&repo).unwrap();
/// // The key is the key of the canonical form, ends at the folder,
/// // and carries no trailing separator (the leading spelling is
/// // platform-shaped: a drive letter on Windows, a `/` elsewhere):
/// assert_eq!(key, path_key(&canonical));
/// assert!(key.ends_with("repo"));
/// assert!(!key.ends_with('/'));
/// ```
pub fn canonical_path_key(path: &Path) -> Result<String, CanonicalizeError> {
    canonicalize(path).map(|canonical| path_key(&canonical))
}

/// The identity key of an ALREADY-CANONICALIZED path (DB-030): the
/// platform's [`PathKeyRule`] applied to the path text — `/`
/// separators, interior and trailing separator runs collapsed, and on
/// Windows/macOS NFC plus simple case folding.
///
/// Total: any input produces a key, never a panic. But DB-030/DB-031's
/// identity promises hold only for canonical input: a non-canonical
/// path gets the key of its TEXT (e.g. `/repo` and a symlink spelled
/// `/alias` key differently until canonicalized), which is why
/// [`canonical_path_key`] is the entry point flows should call. The
/// repository scan (ALG-053) keys the paths it walked; that holds
/// given the scan is rooted at a stored canonical path and keeps
/// D-07's no-follow rule — the scan task owns both preconditions.
///
/// Unix paths are bytes and the key is TEXT: non-UTF-8 path bytes
/// reach the key as U+FFFD replacement characters (the same lossy
/// form logs and dialogs show).
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use eunomia_fs::paths::path_key;
///
/// // The running platform's rule; on Linux the case is kept, on
/// // Windows and macOS it folds. The shape holds everywhere:
/// let key = path_key(Path::new("/Repo/MyProject"));
/// assert!(key.starts_with('/'));
/// assert!(!key.ends_with('/'));
/// ```
#[must_use = "the key is the value; building it and dropping it wastes the allocation"]
pub fn path_key(canonical: &Path) -> String {
    path_key_with_rule(&canonical.to_string_lossy(), platform_rule())
}

/// [`path_key`] with the Unicode rule fed explicitly: the pure,
/// OS-independent core of the key builder, so every platform's
/// behavior is unit-testable everywhere (the Linux machine runs the
/// Windows and macOS rules by passing them in; the per-OS wrappers
/// get their literal pins on the CI matrix).
///
/// The pipeline, per the module docs: separators normalize per rule,
/// then — for [`PathKeyRule::Windows`] and [`PathKeyRule::MacOs`] —
/// NFC, simple case folding, NFC again. For [`PathKeyRule::Linux`]
/// the separator-normalized text IS the key.
///
/// # Examples
///
/// ```
/// use eunomia_fs::paths::{path_key_with_rule, PathKeyRule};
///
/// // The NFC/NFD pair: one name, two spellings, one folding key —
/// // and two byte-distinct keys on Linux, where they are two real
/// // folder names:
/// let nfc = "caf\u{e9}";
/// let nfd = "caf\u{65}\u{301}";
/// assert_eq!(
///     path_key_with_rule(nfc, PathKeyRule::MacOs),
///     path_key_with_rule(nfd, PathKeyRule::MacOs),
/// );
/// assert_ne!(
///     path_key_with_rule(nfc, PathKeyRule::Linux),
///     path_key_with_rule(nfd, PathKeyRule::Linux),
/// );
/// ```
#[must_use = "the key is the value; building it and dropping it wastes the allocation"]
pub fn path_key_with_rule(canonical: &str, rule: PathKeyRule) -> String {
    let keyed = normalize_separators(canonical, rule == PathKeyRule::Windows);
    match rule {
        // Linux: separators only — case kept, bytes kept (DB-030).
        PathKeyRule::Linux => keyed,
        // Windows/macOS: NFC → simple fold → NFC. Folding does not
        // preserve normalization (CaseFolding.txt's header note), so
        // the folded text is normalized again; the leading NFC makes
        // decomposed spellings fold like their precomposed twins.
        PathKeyRule::Windows | PathKeyRule::MacOs => {
            let nfc = |text: &str| text.nfc().collect::<String>();
            nfc(&simple_fold(&nfc(&keyed)))
        }
    }
}

/// The rule of the platform this binary runs on (`cfg!`-selected, so
/// every branch compiles everywhere).
pub(crate) fn platform_rule() -> PathKeyRule {
    if cfg!(target_os = "windows") {
        PathKeyRule::Windows
    } else if cfg!(target_os = "macos") {
        PathKeyRule::MacOs
    } else {
        PathKeyRule::Linux
    }
}

/// Rewrites separators per rule: the Windows rule splits on both `/`
/// and `\` and rejoins with `/` (backslash is never a name byte
/// there); the Linux and macOS rules split on `/` only, so a Unix
/// backslash rides inside its component untouched. Interior and
/// trailing separator runs collapse; a single leading separator (the
/// absolute marker) and a double one (a UNC share prefix) survive.
fn normalize_separators(text: &str, windows_separators: bool) -> String {
    let is_separator = |c: char| c == '/' || (windows_separators && c == '\\');
    let stripped = text.trim_start_matches(is_separator);
    let leading = text.chars().count() - stripped.chars().count();
    let prefix = match leading {
        0 => "",
        1 => "/",
        _ => "//",
    };
    let components: Vec<&str> = stripped
        .split(is_separator)
        .filter(|c| !c.is_empty())
        .collect();
    format!("{prefix}{}", components.join("/"))
}

/// Unicode SIMPLE case folding (DB-030's "1:1 mappings"): every
/// character with a simple fold (CaseFolding.txt statuses C and S)
/// maps to its folded form; characters whose fold is multi-character
/// (ligatures `ﬁ`, `ß`'s full fold to `ss`) have none and stay
/// themselves. `unicode-case-mapping::case_folded` is exactly that
/// function — `None` means "maps to itself".
fn simple_fold(text: &str) -> String {
    text.chars()
        .map(|c| {
            case_folded(c)
                .and_then(|folded| char::from_u32(folded.get()))
                // Unreachable with valid Unicode data (no fold target
                // is a surrogate); the fallback keeps the builder
                // total over any table.
                .unwrap_or(c)
        })
        .collect()
}

/// Why [`canonicalize`] said no: the OS refused to resolve the path.
/// One failure mode (the resolution call itself), so a struct — the
/// input path text and the OS error are the two facts the §10 log
/// line needs, exposed through [`CanonicalizeError::path`] and
/// [`CanonicalizeError::os_code`] the way
/// [`ProjectUnavailableError`] exposes its verdicts. Hand-rolled
/// impls, not `thiserror`: the crate's §5.2 dependency list has no
/// `thiserror` (the DECISIONS.md D-017 store precedent).
#[derive(Debug)]
pub struct CanonicalizeError {
    /// The path text this failure is about — the log line's subject.
    path: String,
    /// The OS refusal, carrying the raw error code.
    source: io::Error,
}

impl CanonicalizeError {
    /// The path text canonicalization was asked for (lossy form of
    /// the input, the same text the log line shows).
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The OS failure behind the refusal; its `raw_os_error` and
    /// `ErrorKind` are §10's first-class log facts.
    #[must_use]
    pub fn io_error(&self) -> &io::Error {
        &self.source
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`) —
    /// §10 names it a first-class fact of every error log line.
    #[must_use]
    pub fn os_code(&self) -> Option<i32> {
        self.source.raw_os_error()
    }
}

impl fmt::Display for CanonicalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot canonicalize project path {}: {}",
            self.path, self.source
        )
    }
}

impl std::error::Error for CanonicalizeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use tempfile::TempDir;

    /// The fixture's owner uid is this process's effective uid (the
    /// fixture was created by it): uid 0 means the tests run as root,
    /// where mode bits are not enforced and a chmod-000 directory
    /// stays listable — the precondition of a permission test cannot
    /// be set up. The caller skips loudly instead of passing
    /// vacuously (precedent: `running_as_root_is_untestable` in
    /// eunomia-store).
    #[cfg(unix)]
    fn running_as_root_is_untestable(temp: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(temp)
            .expect("the fixture directory must be statable")
            .uid()
            == 0
    }

    fn repo_with_git_entry(dir: &Path, name: &str) -> std::path::PathBuf {
        let repo = dir.join(name);
        fs::create_dir_all(repo.join(".git")).expect("creating the fixture repository must work");
        repo
    }

    #[test]
    fn a_folder_with_a_git_directory_is_available() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");

        check_project_available(&repo)
            .expect("a folder with a .git directory is the happy path of the §6.9 row");
    }

    #[test]
    fn a_folder_with_a_git_file_is_available() {
        // The worktree/submodule form: `.git` is a FILE holding a
        // `gitdir:` pointer (ALG-052's other form). Availability needs
        // only "there is a .git entry" — the scan's skip of the file
        // form is a scan rule, not an availability rule.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("worktree");
        fs::create_dir(&repo).expect("creating the fixture repository must work");
        fs::write(repo.join(".git"), "gitdir: ../main/repo/.git\n")
            .expect("writing the gitdir pointer must work");

        check_project_available(&repo)
            .expect("a .git FILE must be accepted exactly like a .git directory");
    }

    #[test]
    fn a_plain_file_is_reported_not_a_directory() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let file = dir.path().join("plain.txt");
        fs::write(&file, b"not a folder").expect("writing the fixture file must work");

        let err = check_project_available(&file)
            .expect_err("a plain file must fail the is-a-folder half of the §6.9 row");
        assert!(
            matches!(&err, ProjectUnavailableError::NotADirectory),
            "a plain file must be NotADirectory, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            None,
            "the stat succeeded, so no OS code exists for this verdict"
        );
    }

    #[test]
    fn a_missing_path_is_reported_missing() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let gone = dir.path().join("never-created");

        let err = check_project_available(&gone)
            .expect_err("a path that does not exist must be unavailable (D-09)");
        assert_eq!(
            err.os_code().map(|_| ()),
            Some(()),
            "an io-backed verdict carries an OS code"
        );
        match &err {
            ProjectUnavailableError::Missing { source } => {
                assert_eq!(
                    source.kind(),
                    io::ErrorKind::NotFound,
                    "a vanished folder is ENOENT"
                );
                #[cfg(unix)]
                assert_eq!(
                    source.raw_os_error(),
                    Some(2),
                    "ENOENT is the OS code the §10 log line carries"
                );
            }
            other => panic!("a missing path must be Missing, got: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_without_read_permission_is_reported_not_readable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        if running_as_root_is_untestable(&repo) {
            eprintln!(
                "skipping a_folder_without_read_permission_is_reported_not_readable: \
                 running as root — mode bits are not enforced, the \
                 unreadable-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&repo, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let result = check_project_available(&repo);
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(&repo, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let err = result.expect_err("a folder that cannot be listed must be unavailable");
        assert!(
            matches!(&err, ProjectUnavailableError::NotReadable { .. }),
            "a 000-mode folder must be NotReadable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(13),
            "EACCES is the OS code the §10 log line carries for denied access"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_readable_but_unsearchable_folder_surfaces_the_denied_git_stat() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        if running_as_root_is_untestable(&repo) {
            eprintln!(
                "skipping a_readable_but_unsearchable_folder_surfaces_the_denied_git_stat: \
                 running as root — mode bits are not enforced, the \
                 unsearchable-folder precondition cannot be set up"
            );
            return;
        }

        // r without x: the probe read (listing) succeeds, but the
        // `.git` entry cannot be statted — the second NotReadable
        // producer, distinct from the denied listing above.
        fs::set_permissions(&repo, fs::Permissions::from_mode(0o400))
            .expect("removing the search bits must work");
        // Prove the failure below comes from the `.git` stat and not
        // from an earlier probe: with mode 0o400 the listing itself
        // must still succeed. The check is captured while the fixture
        // is locked but asserted only after the restore, so even a
        // precondition panic cannot strand an unremovable fixture.
        let listing_still_ok = fs::read_dir(&repo).is_ok();
        let result = check_project_available(&repo);
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(&repo, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");
        assert!(
            listing_still_ok,
            "precondition: a 0o400 folder must still be listable"
        );

        let err =
            result.expect_err("a folder whose .git entry cannot be statted must be unavailable");
        assert!(
            matches!(&err, ProjectUnavailableError::NotReadable { .. }),
            "a denied .git stat must be NotReadable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(13),
            "EACCES is the OS code the §10 log line carries for denied access"
        );
    }

    #[test]
    fn a_vanished_git_entry_flips_the_verdict() {
        // The card's TOCTOU acceptance case: the folder HAS `.git`,
        // the check passes, the entry disappears, the next check
        // reports the folder unavailable — the verdict is what the
        // probes observed, not a cached snapshot.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        check_project_available(&repo).expect("the repository is available while .git exists");

        fs::remove_dir_all(repo.join(".git")).expect("removing the .git entry must work");
        let err = check_project_available(&repo)
            .expect_err("after .git disappears the folder must be unavailable");
        assert!(
            matches!(&err, ProjectUnavailableError::NoGitEntry),
            "a folder without .git must be NoGitEntry, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            None,
            "the folder itself is readable; no OS call failed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_git_symlink_is_still_a_git_entry() {
        // Presence, not validity: a `.git` symlink counts as the
        // entry, and whether it points at a healthy gitdir is git's
        // business, not this check's.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("linked");
        fs::create_dir(&repo).expect("creating the fixture repository must work");
        std::os::unix::fs::symlink("/nowhere/gitdir", repo.join(".git"))
            .expect("creating the .git symlink must work");

        check_project_available(&repo).expect("a .git entry that is a symlink counts");
    }

    #[cfg(unix)]
    #[test]
    fn io_backed_failures_carry_the_raw_os_error_code() {
        // The `.git` probe under a path whose parent is a plain file:
        // ENOTDIR, not ENOENT — classified NotReadable (the path
        // cannot be a folder), with the errno riding in the typed
        // error so the §10 log line keeps its OS code even where the
        // reason bucket is coarse. Runs regardless of privileges,
        // unlike the permission tests.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let file = dir.path().join("plain.txt");
        fs::write(&file, b"not a folder").expect("writing the fixture file must work");

        let err = check_project_available(&file.join(".git"))
            .expect_err("a .git probe under a plain file must fail");
        assert!(
            matches!(&err, ProjectUnavailableError::NotReadable { .. }),
            "ENOTDIR must be NotReadable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(20),
            "ENOTDIR = 20 must ride into the typed error"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_project_root_checks_through_to_the_repository() {
        // The root stat follows symlinks: a project registered as a
        // link to the real repository checks the repository. (DB-031's
        // aliasing of two canonical paths is the canonicalization
        // task's concern; availability simply follows the link.)
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&repo, &alias).expect("creating the root symlink must work");

        check_project_available(&alias)
            .expect("a symlinked project root checks through to the repository");

        // A link whose target is gone is the "vanished folder" shape:
        // ENOENT at the stat, reported as Missing.
        let dangling = dir.path().join("dangling");
        std::os::unix::fs::symlink(dir.path().join("never-created"), &dangling)
            .expect("creating the dangling symlink must work");
        let err = check_project_available(&dangling)
            .expect_err("a dangling root link must be unavailable");
        assert!(
            matches!(&err, ProjectUnavailableError::Missing { .. }),
            "a dangling root link must be Missing, got: {err:?}"
        );
    }

    #[test]
    fn display_texts_and_the_source_chain_follow_the_contract() {
        // The command layer branches on the variants and logs the
        // Display text; these assertions pin the wording (lowercase,
        // no trailing punctuation — the house style) and the source()
        // chain without pinning std's strerror table.
        let not_a_directory = ProjectUnavailableError::NotADirectory;
        assert_eq!(
            not_a_directory.to_string(),
            "project path is not a directory"
        );
        let no_git_entry = ProjectUnavailableError::NoGitEntry;
        assert_eq!(
            no_git_entry.to_string(),
            "project folder contains no .git entry"
        );

        let missing = ProjectUnavailableError::Missing {
            source: io::Error::from_raw_os_error(13),
        };
        assert!(
            missing
                .to_string()
                .starts_with("project folder does not exist: "),
            "the Missing text must state the fact, then the OS text: {}",
            missing
        );
        let not_readable = ProjectUnavailableError::NotReadable {
            source: io::Error::from_raw_os_error(13),
        };
        assert!(
            not_readable
                .to_string()
                .starts_with("project folder cannot be read: "),
            "the NotReadable text must state the fact, then the OS text: {}",
            not_readable
        );

        // source() chains exactly to the carried io error on the
        // io-backed variants, and is None where no OS call failed.
        assert_eq!(
            missing.source().map(|source| source.to_string()),
            Some(io::Error::from_raw_os_error(13).to_string())
        );
        assert!(not_readable.source().is_some());
        assert!(not_a_directory.source().is_none());
        assert!(no_git_entry.source().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_on_another_filesystem_is_available() {
        use std::os::unix::fs::MetadataExt;

        // DB-032: network and removable drives are allowed. A real
        // network share cannot be mounted from a unit test; the honest
        // stand-in is a fixture on a filesystem DIFFERENT from the
        // test's own temp dir: `/dev/shm` is a separate tmpfs mount
        // with its own st_dev. A drive-type heuristic — the "reject
        // non-local volumes" shortcut this contract forbids — keys on
        // exactly that mount identity, so a heuristic would reject
        // this fixture; the four-probe check must accept it.
        let shm = Path::new("/dev/shm");
        let Ok(shm_metadata) = fs::metadata(shm) else {
            eprintln!(
                "skipping a_folder_on_another_filesystem_is_available: \
                 no /dev/shm on this system, no cross-filesystem fixture possible"
            );
            return;
        };
        let temp_dev = fs::metadata(std::env::temp_dir())
            .expect("the temp directory must be statable")
            .dev();
        if shm_metadata.dev() == temp_dev {
            eprintln!(
                "skipping a_folder_on_another_filesystem_is_available: \
                 /dev/shm sits on the same filesystem as the temp dir, \
                 no cross-filesystem fixture possible"
            );
            return;
        }

        let mount = match tempfile::Builder::new()
            .prefix("eunomia-fs-t060-crossfs-")
            .tempdir_in(shm)
        {
            Ok(mount) => mount,
            Err(error) => {
                eprintln!(
                    "skipping a_folder_on_another_filesystem_is_available: \
                     a fixture cannot be created under /dev/shm ({error}), \
                     no cross-filesystem fixture possible"
                );
                return;
            }
        };
        let repo = mount.path().join("repo");
        fs::create_dir_all(repo.join(".git")).expect("creating the fixture repository must work");

        check_project_available(&repo).expect(
            "a repository on another filesystem must be available: \
             DB-032 allows non-local drives, and the check consults \
             only existence, type, readability and the .git entry",
        );
    }

    #[test]
    fn a_path_shaped_like_a_scan_excluded_name_is_available() {
        // ALG-051's exclusion names belong to the repository SCAN; a
        // "reject mount-point-shaped paths" shortcut would match the
        // `mnt` component. Availability's contract is only the four
        // probes, so a repository stored under either shape is checked
        // like any other folder — the same DB-032 "no path-shape
        // sniffing" promise as the cross-filesystem test, exercised on
        // path TEXT instead of mount identity.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let recycled = dir.path().join("$RECYCLE.BIN").join("repo");
        fs::create_dir_all(recycled.join(".git"))
            .expect("creating the fixture repository must work");
        let mnt = dir.path().join("mnt").join("repo");
        fs::create_dir_all(mnt.join(".git")).expect("creating the fixture repository must work");

        check_project_available(&recycled)
            .expect("availability must not reuse the scan's exclusion names");
        check_project_available(&mnt).expect("availability must not sniff path shapes");
    }

    // ---- DB-030: canonicalization ------------------------------------

    /// Creates a symlink fixture and reports "untestable" ONLY for the
    /// errors that mean the FILESYSTEM cannot represent symlinks — a
    /// capability absence (`Unsupported`; EPERM = 1 where mounts
    /// disable symlinks, ENOSYS = 38, ENOTSUP/EOPNOTSUPP = 95).
    /// Any other failure (ENOSPC, EIO, EACCES on a directory the test
    /// itself created) is a real setup bug and panics, so DB-031
    /// coverage cannot silently vanish from a CI leg.
    #[cfg(unix)]
    fn symlink_or_report_untestable(target: &Path, link: &Path) -> bool {
        match std::os::unix::fs::symlink(target, link) {
            Ok(()) => true,
            Err(error)
                if error.kind() == io::ErrorKind::Unsupported
                    || matches!(error.raw_os_error(), Some(1 | 38 | 95)) =>
            {
                eprintln!(
                    "skipping a symlink test: the fixture filesystem cannot \
                     represent symlinks ({error}), the property cannot be \
                     set up here"
                );
                false
            }
            Err(error) => panic!("creating the symlink fixture failed: {error}"),
        }
    }

    #[test]
    fn canonicalize_resolves_dotted_paths_to_the_same_absolute_form() {
        // A path spelled with `.` and `..` components names the same
        // folder as the direct spelling: one canonical form.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).expect("creating the fixture repository must work");

        let direct = canonicalize(&repo).expect("an existing folder must canonicalize");
        let parent = direct.parent().expect("a canonical path has a parent");
        let dotted = parent.join(".").join("sub-dotted").join("..").join("repo");
        fs::create_dir(parent.join("sub-dotted")).expect("creating the .. anchor must work");

        assert_eq!(
            canonicalize(&dotted).expect("a dotted path to an existing folder must canonicalize"),
            direct,
            "`.`, `..` and re-joining must not change the canonical form"
        );
        assert!(
            direct.is_absolute(),
            "the canonical form is absolute: {direct:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_resolves_a_symlink_to_the_target_form() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).expect("creating the fixture repository must work");
        let alias = dir.path().join("alias");
        if !symlink_or_report_untestable(&repo, &alias) {
            return;
        }

        assert_eq!(
            canonicalize(&alias).expect("the symlink target exists, canonicalization must work"),
            canonicalize(&repo).expect("the target folder must canonicalize"),
            "the link resolves to its target's canonical form"
        );
        assert_ne!(
            canonicalize(&alias).expect("the symlink target exists, canonicalization must work"),
            alias,
            "the canonical form is the resolved path, not the link spelling"
        );
    }

    #[test]
    fn canonicalize_failure_is_typed_and_carries_path_and_os_code() {
        let missing = Path::new("/definitely/not/here");
        let error = canonicalize(missing)
            .expect_err("a path that does not exist must be a typed failure, not a panic");

        assert_eq!(
            error.path(),
            "/definitely/not/here",
            "the error names the path it was asked about (the §10 log line's subject)"
        );
        assert!(
            error.os_code().is_some(),
            "an OS refusal carries its raw code"
        );
        #[cfg(unix)]
        assert_eq!(
            error.os_code(),
            Some(2),
            "ENOENT is the code the §10 log line carries for a missing folder"
        );
        assert!(
            error
                .to_string()
                .starts_with("cannot canonicalize project path "),
            "the Display text states the fact, then the OS text: {error}"
        );
        assert!(
            error.source().is_some(),
            "the OS error rides in the source chain"
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonicalizing_under_a_plain_file_reports_enotdir() {
        // The "# Errors" clause "a component is not a directory": the
        // OS answers ENOTDIR (20), classified as any other OS refusal.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let file = dir.path().join("plain.txt");
        fs::write(&file, b"not a folder").expect("writing the fixture file must work");

        let error = canonicalize(&file.join(".git"))
            .expect_err("a path under a plain file must fail canonicalization");
        assert_eq!(
            error.os_code(),
            Some(20),
            "ENOTDIR is the code for a non-directory component"
        );
    }

    // ---- DB-030: the stored path is verbatim (owner ruling) ----------

    #[cfg(unix)]
    #[test]
    fn the_stored_canonical_path_carries_the_filesystem_bytes_unchanged() {
        // Reading A (owner ruling, 2026-11-19): NFC and folding apply
        // to the KEY only. A byte-preserving filesystem hands out the
        // name as stored — a decomposed spelling stays decomposed in
        // the canonical path (the KEY would fold it; that is
        // path_key's concern, pinned separately).
        use std::os::unix::ffi::OsStrExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        // "cafe" + COMBINING ACUTE — the NFD spelling of café, never
        // composed by us:
        let decomposed = "caf\u{65}\u{301}";
        let repo = dir.path().join(decomposed);
        fs::create_dir(&repo).expect("creating the fixture repository must work");

        let canonical = canonicalize(&repo).expect("the fixture folder must canonicalize");
        let name = canonical
            .file_name()
            .expect("a canonical path to a folder has a file name");
        assert_eq!(
            name.as_bytes(),
            decomposed.as_bytes(),
            "the stored canonical path must be the verbatim filesystem name, not its NFC form"
        );
    }

    // ---- DB-030: the key shape, per rule (pure, runs everywhere) -----

    #[test]
    fn ascii_case_folds_only_where_the_rule_says() {
        assert_eq!(
            path_key_with_rule("/Repo/MyProject", PathKeyRule::Windows),
            "/repo/myproject"
        );
        assert_eq!(
            path_key_with_rule("/Repo/MyProject", PathKeyRule::MacOs),
            "/repo/myproject"
        );
        assert_eq!(
            path_key_with_rule("/Repo/MyProject", PathKeyRule::Linux),
            "/Repo/MyProject",
            "Linux keeps case (DB-030's last clause)"
        );
    }

    #[test]
    fn windows_keys_rewrite_both_separators_and_drop_the_trailing_one() {
        // `\` is a separator on Windows; `/` keys per DB-030:
        assert_eq!(
            path_key_with_rule("C:\\Repo\\Sub\\", PathKeyRule::Windows),
            "c:/repo/sub"
        );
        assert_eq!(
            path_key_with_rule("C:\\", PathKeyRule::Windows),
            "c:",
            "a trailing separator is not part of the key"
        );
    }

    #[test]
    fn unix_backslash_is_a_name_byte_not_a_separator() {
        // On Unix, `\` is a legal character INSIDE a folder name: the
        // Linux and macOS rules must not treat it as a separator, or
        // two distinct folders (`a\b` vs `a/b`) would collide on one
        // key. The identity the whole key exists for:
        assert_eq!(path_key_with_rule("/x/a\\b", PathKeyRule::Linux), "/x/a\\b");
        assert_ne!(
            path_key_with_rule("/x/a\\b", PathKeyRule::Linux),
            path_key_with_rule("/x/a/b", PathKeyRule::Linux),
        );
        assert_eq!(path_key_with_rule("/x/a\\b", PathKeyRule::MacOs), "/x/a\\b");
    }

    #[test]
    fn separator_runs_collapse_and_the_leading_markers_survive() {
        // Interior and trailing runs collapse; the absolute marker and
        // the UNC double marker stay (they distinguish paths):
        assert_eq!(
            path_key_with_rule("/repo//sub///", PathKeyRule::Linux),
            "/repo/sub"
        );
        assert_eq!(
            path_key_with_rule("/", PathKeyRule::Linux),
            "/",
            "the filesystem root's key is the root, not an empty string"
        );
        assert_eq!(
            path_key_with_rule("//server/share/Repo", PathKeyRule::Windows),
            "//server/share/repo",
            "a UNC share prefix keeps its double marker (the pure \
             builder's shape rule; the verbatim form dunce hands back \
             for real UNC shares is pinned separately below)"
        );
        // The spelling production actually reaches for a UNC share:
        // dunce cannot safely strip `\\?\UNC\`, so the canonical path
        // keeps the verbatim prefix and the key carries its components
        // (deterministic, and pinned here so the real shape cannot
        // drift silently):
        assert_eq!(
            path_key_with_rule("\\\\?\\UNC\\server\\share\\Repo", PathKeyRule::Windows),
            "//?/unc/server/share/repo",
        );
    }

    #[test]
    fn nfc_and_nfd_spellings_of_one_name_collapse_under_the_folding_rules() {
        // The macOS/APFS situation, built purely: one name spelled
        // precomposed and decomposed. The folding rules must produce
        // ONE key (and Linux must keep them distinct — there they are
        // two different folder names).
        let pairs = [
            ("caf\u{e9}", "caf\u{65}\u{301}"), // é composed vs e + COMBINING ACUTE
            ("\u{c9}cole", "E\u{301}cole"),    // É composed vs E + COMBINING ACUTE
        ];
        for (precomposed, decomposed) in pairs {
            assert_ne!(precomposed, decomposed);
            assert_eq!(
                path_key_with_rule(precomposed, PathKeyRule::MacOs),
                path_key_with_rule(decomposed, PathKeyRule::MacOs),
                "NFC and NFD spellings must share one folding key: {precomposed:?}"
            );
            assert_eq!(
                path_key_with_rule(precomposed, PathKeyRule::Windows),
                path_key_with_rule(decomposed, PathKeyRule::Windows),
                "NFC and NFD spellings must share one folding key: {precomposed:?}"
            );
            assert_ne!(
                path_key_with_rule(precomposed, PathKeyRule::Linux),
                path_key_with_rule(decomposed, PathKeyRule::Linux),
                "Linux normalizes nothing: the two byte-names stay distinct"
            );
        }
    }

    // ---- DB-030: simple-fold fidelity (the owner ruling, both sides) --

    #[test]
    fn ligatures_without_simple_folds_stay_distinct_from_their_expansions() {
        // The ruling's NO side: simple folding is 1:1 only. Full
        // folding would merge ﬁle with FIle and straße with STRASSE —
        // two folders the filesystems keep distinct — and must never
        // happen. Windows rule shown; macOS shares the body (pinned
        // in the rule tests above and on the CI matrix).
        let win = PathKeyRule::Windows;
        // ﬁ (U+FB01) has an F-only entry: it does not fold:
        assert_ne!(
            path_key_with_rule("\u{fb01}le", win),
            path_key_with_rule("FIle", win),
            "no ligature unfolding: ﬁle and FIle are distinct keys"
        );
        // ß (U+00DF) is F-only too: ß never becomes ss:
        assert_ne!(
            path_key_with_rule("stra\u{df}e", win),
            path_key_with_rule("STRASSE", win),
            "no ß→ss unfolding: straße and STRASSE are distinct keys"
        );
        // İ (U+0130) has only F/T entries: it stays itself, distinct
        // from a folded capital I:
        assert_ne!(
            path_key_with_rule("\u{130}stanbul", win),
            path_key_with_rule("Istanbul", win),
            "İ has no simple fold: İstanbul and Istanbul are distinct keys"
        );
    }

    #[test]
    fn one_to_one_mappings_do_fold_per_the_ruling() {
        // The ruling's YES side: 1:1 mappings fold, exactly where
        // NTFS $UpCase and APFS fold too.
        let win = PathKeyRule::Windows;
        // ſ (U+017F) — the C entry the ruling confirmed (ſ→s):
        assert_eq!(
            path_key_with_rule("\u{17f}ong", win),
            path_key_with_rule("song", win),
            "ſ folds to s (1:1 mapping)"
        );
        // ẞ (U+1E9E) — the S entry: ẞ→ß, while ß itself stays (its
        // fold is F-only) — the pair that separates simple from both
        // full folding and any naive lowercase:
        assert_eq!(
            path_key_with_rule("\u{1e9e}", win),
            path_key_with_rule("\u{df}", win),
            "ẞ folds to ß (the S entry)"
        );
        assert_ne!(
            path_key_with_rule("\u{df}", win),
            path_key_with_rule("SS", win),
            "and ß stays distinct from ss"
        );
        // KELVIN SIGN (U+212A) — the C entry that also shrinks length:
        assert_eq!(
            path_key_with_rule("\u{212a}elvin", win),
            path_key_with_rule("Kelvin", win),
            "KELVIN SIGN folds to k (1:1 mapping)"
        );
        // The same folds under the macOS rule (shared body, separate
        // variant — one fold each pins that the variant is wired):
        assert_eq!(path_key_with_rule("\u{17f}ong", PathKeyRule::MacOs), "song");
        assert_eq!(
            path_key_with_rule("\u{1e9e}", PathKeyRule::MacOs),
            path_key_with_rule("\u{df}", PathKeyRule::MacOs)
        );
    }

    #[test]
    fn the_key_pipeline_is_a_fixed_point_under_reapplication() {
        // The key of a key is the key: stored keys re-key identically
        // when a future flow feeds one back through the builder.
        let texts = [
            "/Repo/MyProject",
            "C:\\Repo\\Sub\\",
            "//server/share/Repo",
            "caf\u{65}\u{301}",  // decomposed spelling
            "\u{17f}tra\u{df}e", // ſtraße
            "\u{fb01}le",        // ﬁle
            "\u{212a}elvin",     // KELVIN SIGN
            "/",
            "//server",
            "",
        ];
        for text in texts {
            for rule in [PathKeyRule::Linux, PathKeyRule::Windows, PathKeyRule::MacOs] {
                let once = path_key_with_rule(text, rule);
                let twice = path_key_with_rule(&once, rule);
                assert_eq!(
                    once, twice,
                    "the key must be a fixed point: {text:?} under {rule:?}"
                );
            }
        }
    }

    #[test]
    fn the_key_builder_is_total_over_arbitrary_text() {
        // Total posture: no input panics, and the shape invariants
        // hold for every input (determinism is structural — a pure
        // function of its inputs — and covered by the fixed-point
        // test above).
        let texts = [
            "",
            "/",
            "//",
            "///",
            "a\u{0}b",
            "\u{301}", // a bare combining mark
            "İ\u{0}\\/",
            "🚀",
            &"x".repeat(10_000),
        ];
        for text in texts {
            for rule in [PathKeyRule::Linux, PathKeyRule::Windows, PathKeyRule::MacOs] {
                let key = path_key_with_rule(text, rule);
                assert!(
                    key == "/" || key == "//" || !key.ends_with('/'),
                    "no trailing separator survives: {text:?} → {key:?}"
                );
                if rule == PathKeyRule::Windows {
                    assert!(
                        !key.contains('\\'),
                        "the Windows rule rewrites every backslash: {text:?} → {key:?}"
                    );
                }
            }
        }
        assert_eq!(
            path_key_with_rule("", PathKeyRule::Linux),
            "",
            "the empty text keys to the empty string — the builder never panics"
        );
    }

    // ---- DB-030: the platform wrapper ---------------------------------

    #[test]
    #[cfg(target_os = "linux")]
    fn on_linux_the_platform_key_keeps_case() {
        assert_eq!(path_key(Path::new("/Repo/MyProject")), "/Repo/MyProject");
    }

    #[test]
    #[cfg(windows)]
    fn on_windows_the_platform_key_folds_and_slashes() {
        assert_eq!(
            path_key(Path::new("C:\\Repo\\MyProject")),
            "c:/repo/myproject"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn on_macos_the_platform_key_folds() {
        assert_eq!(path_key(Path::new("/Repo/MyProject")), "/repo/myproject");
    }

    // ---- DB-030/DB-031: the Windows-leg tests (CI matrix) -------------

    #[test]
    #[cfg(windows)]
    fn on_windows_canonicalization_strips_the_verbatim_prefix_from_plain_drives() {
        // DB-030's prefix clause, on the one OS where it means
        // something: a plain drive path (dunce's safe-to-strip case)
        // comes back without `\\?\`, absolute.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).expect("creating the fixture repository must work");

        let canonical = canonicalize(&repo).expect("the fixture folder must canonicalize");
        let text = canonical.to_string_lossy();
        assert!(
            !text.starts_with(r"\\?\"),
            "a plain drive path must not keep the verbatim prefix: {text:?}"
        );
        assert!(canonical.is_absolute());

        // The key shape of a stripped drive path: folded, `/`
        // separators, drive letter first:
        let key = canonical_path_key(&repo).expect("the fixture folder must canonicalize");
        assert_eq!(key, path_key(&canonical));
        assert!(
            key.len() >= 3 && key.as_bytes()[1] == b':' && key.as_bytes()[2] == b'/',
            "the key of a stripped drive path is <letter>:/…: {key:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn on_windows_a_folder_and_its_directory_symlink_yield_one_path_key() {
        // The DB-031 acceptance on Windows: canonicalization goes
        // through GetFinalPathNameByHandle, the resolution path where
        // Windows can differ from Unix. Directory symlinks need
        // SeCreateSymbolicLinkPrivilege: only that capability absence
        // skips (ERROR_PRIVILEGE_NOT_HELD = 1314); anything else is a
        // real setup failure and panics.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).expect("creating the fixture repository must work");
        let alias = dir.path().join("alias");
        if let Err(error) = std::os::windows::fs::symlink_dir(&repo, &alias) {
            if error.raw_os_error() == Some(1314) {
                eprintln!(
                    "skipping the Windows DB-031 test: the runner lacks \
                     SeCreateSymbolicLinkPrivilege ({error}), the symlink \
                     fixture cannot be created here"
                );
                return;
            }
            panic!("creating the directory-symlink fixture failed: {error}");
        }

        assert_ne!(
            path_key(&repo),
            path_key(&alias),
            "precondition: the two TEXT spellings key differently — only \
             canonicalization collapses the alias (DB-031)"
        );
        assert_eq!(
            canonical_path_key(&repo).expect("the real folder must canonicalize"),
            canonical_path_key(&alias).expect("the symlink resolves to the same folder"),
            "DB-031: a folder and a symlink to it count as ONE project"
        );
    }

    // ---- DB-030: TEXT keys over byte paths ----------------------------

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_bytes_reach_the_key_as_replacement_characters() {
        // Unix paths are bytes, the key is TEXT: the lossy conversion
        // is part of the contract, pinned so a silent switch to panic
        // or drop cannot slip in.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(b"/tmp/\xff\xfe"));
        let key = path_key(path);
        assert!(
            key.contains('\u{fffd}'),
            "invalid bytes surface as U+FFFD in the key: {key:?}"
        );
    }

    // ---- DB-031: the symlink acceptance test --------------------------

    #[cfg(unix)]
    #[test]
    fn a_folder_and_its_symlink_yield_one_path_key() {
        // THE card's acceptance case: one real folder, a symlink
        // pointing at it — both paths yield the SAME key, because
        // canonicalization resolves the link before keying (DB-031).
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).expect("creating the fixture repository must work");
        let alias = dir.path().join("alias");
        if !symlink_or_report_untestable(&repo, &alias) {
            return;
        }

        assert_ne!(
            path_key(&repo),
            path_key(&alias),
            "precondition: the two TEXT spellings key differently — only \
             canonicalization collapses the alias (DB-031)"
        );
        assert_eq!(
            canonical_path_key(&repo).expect("the real folder must canonicalize"),
            canonical_path_key(&alias).expect("the symlink resolves to the same folder"),
            "DB-031: a folder and a symlink to it count as ONE project"
        );
        // The same equality through the two halves separately — the
        // resolution is canonicalize's, the keying is total on it:
        assert_eq!(
            path_key(&canonicalize(&repo).expect("the real folder must canonicalize")),
            path_key(&canonicalize(&alias).expect("the symlink resolves to the same folder")),
        );
    }
}
