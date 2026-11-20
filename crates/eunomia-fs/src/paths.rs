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
//! preflight step 1), the project-path canonicalization with the
//! `path_key` identity key (DB-030, DB-031, D-07), the Git
//! repository root lookup ([`find_git_root`], PRJ-003's walk-up from a
//! chosen subfolder, D-08), and the sync target-path resolution with
//! its symlink checks ([`resolve_target_path`], 9.4 preflight step 2,
//! 9.5 step 1, T-F6). Still planned: the SEC-003 allowlist path
//! checks.
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
//! test. The repository-root lookup is tested through the public
//! function against real fixtures: the picked folder being the root
//! itself, a subfolder two levels deep, the nearest `.git` winning over
//! an outer nested repository, the `.git`-file worktree form and a
//! `.git` symlink as root terminators, a symlinked input resolving to
//! the real root, the typed negative lookup outside any repository
//! (with the temp dir's real ancestors verified `.git`-free
//! independently of the function under test), and the refusal
//! classifications — a denied probe under a folder without the search
//! bit (loud root skip), a missing input, and a plain-file input. The
//! SEC-003 checks keep their planned coverage: rejection of
//! paths outside the registered or session-allowed set. The
//! target-path resolution is tested through the public function
//! against real fixtures: the absent-target creation case and the
//! regular-file pass-through, the `CLAUDE.md` → `AGENTS.md` acceptance
//! (the write target is the real file and the resolved name is its
//! name), a depth-2 link chain, the dangling link and the loop, the
//! outside-root containment (a link to a real file outside the fixture
//! root), the folder verdicts for a plain folder and for a link
//! resolving through to one, a relative link spelling, the structural
//! escape refusals (`..` and absolute spellings are refused —
//! the textual prefix would match, so the check demands ordinary
//! names; an interior `.` is inert and pinned accepted), the
//! intermediate-link refusals (a deeper name reached through — or
//! created behind — a link hiding in a middle component judges by the
//! real location), the containment
//! posture (byte-exact pinned on Linux; the Windows fold retry pinned
//! by the cfg-gated leg and by the pure parameterized core), a denied
//! target probe (loud root skip), the observed-tree flip of the
//! verdict, and the error-contract pin.

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

    /// Consumes the error into the OS failure behind it — for typed
    /// errors that classify the refusal by `ErrorKind`/code and then
    /// re-raise the io error as their own variant (the
    /// [`TargetPathError`] resolution verdicts; [`NotARepoError`]
    /// carries the [`CanonicalizeError`] whole instead).
    #[must_use]
    pub fn into_io_error(self) -> io::Error {
        self.source
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

// ---- PRJ-003 / D-08: the Git repository root lookup ----------------

/// Finds the root of the Git repository the chosen folder belongs to
/// (PRJ-003, D-08): the folder picked in the add-project flow may sit
/// anywhere inside a repository, and the app must offer the repository
/// root. The input is canonicalized FIRST — the module's own
/// [`canonicalize`], the D-086 posture, so the returned root is the
/// canonical path the app stores (DB-030) — and then the walk climbs
/// from the canonical path through its ancestors, probing each
/// directory for a `.git` entry. The FIRST `.git` found — the nearest
/// root — terminates the walk, and that directory is the answer.
///
/// # What counts as a root
///
/// ANY entry named `.git`: a directory (a normal repository), a file
/// (a worktree/submodule gitdir pointer — ALG-052's other form), or a
/// symlink to either. Presence, not validity — the D-084 idiom, the
/// same probe [`check_project_available`] makes for the §6.9 row
/// ("folder with `.git` (folder or file)"). ALG-052's distinction
/// between the forms belongs to the repository SCAN, which skips the
/// file form with the reason "submodule or worktree"; this function
/// serves the add-by-hand flow, where §6.9 and D-06 both accept the
/// file form ("you can add it by hand") — so a linked worktree's
/// folder resolves to the worktree's own root, never through the
/// pointer into the main repository. The probe is
/// `symlink_metadata(<dir>/.git)`: it inspects the entry itself
/// without following it, so even a dangling `.git` symlink is a root.
///
/// # Nearest root wins
///
/// The walk is bottom-up and stops at the first `.git`, so a folder
/// inside a nested repository (D-06: nested repositories are
/// registered as separate projects) resolves to the inner repository,
/// never to the outer one.
///
/// # Termination is structural
///
/// [`Path::ancestors`] yields the canonical path, then its successive
/// parents, and stops at the filesystem root (whose `.git` probe still
/// runs — a repository mounted at `/` is found). Every ancestor of a
/// canonical path is a strict prefix of it, so the walk visits each
/// level exactly once and cannot cycle: symlink loops cannot re-enter,
/// because canonicalization resolved every link before the walk began
/// (D-07 converts a hand-picked path to its canonical form). No depth
/// limit and no configuration: the walk is bounded by the path
/// itself, one `symlink_metadata` per level.
///
/// # Races are honest (TOCTOU)
///
/// The probes are separate OS calls over a live filesystem: between
/// the canonicalization and any probe the tree can change — a level
/// can lose or gain its `.git`, permissions on a walked folder can
/// change. The function reports what the probes observed at their
/// moment; no snapshot is taken and no atomicity is pretended. A probe
/// that is REFUSED (denied search on a walked folder — which also
/// covers an ancestor denied in the walk's window; the canonicalized
/// input being a plain file, probed at `<file>/.git`; a drive dropped
/// mid-walk) is a typed [`NotARepoError::NotReadable`] carrying the
/// refused path and the OS error — NEVER a silent "no repository":
/// "could not check" is not "checked and absent", and §10 keeps the
/// two apart (`E_NOT_A_REPO` invites "Pick the repository root"; an
/// access failure does not). A plain `NotFound` means "no repository
/// at this level" and the walk continues.
///
/// # Errors
///
/// [`NotARepoError`], one variant per failure mode:
/// [`NotARepoError::NoGitEntry`] is the negative lookup — every level
/// up to the filesystem root was probed and none has a `.git` entry.
/// No OS call failed, so the variant carries no [`io::Error`] and
/// [`NotARepoError::os_code`] answers `None`: a fact about the tree,
/// not an OS refusal — and the variant the future `E_NOT_A_REPO`
/// mapping consumes (the command layer maps it; D-062's posture keeps
/// `AppError` out of this crate). [`NotARepoError::NotReadable`] and
/// [`NotARepoError::Canonicalize`] carry the OS failure for §10's log
/// facts.
///
/// # Examples
///
/// ```
/// use eunomia_fs::paths::{canonicalize, find_git_root};
///
/// let dir = tempfile::tempdir().unwrap();
/// let repo = dir.path().join("repo");
/// std::fs::create_dir(&repo).unwrap();
/// std::fs::create_dir(repo.join(".git")).unwrap();
/// let deep = repo.join("src").join("fs");
/// std::fs::create_dir_all(&deep).unwrap();
///
/// let canonical_repo = canonicalize(&repo).unwrap();
/// // A folder two levels inside the repository resolves to the root,
/// // and the root itself is its own root:
/// assert_eq!(find_git_root(&deep).unwrap(), canonical_repo);
/// assert_eq!(find_git_root(&repo).unwrap(), canonical_repo);
/// ```
pub fn find_git_root(path: &Path) -> Result<std::path::PathBuf, NotARepoError> {
    let canonical = canonicalize(path).map_err(|source| NotARepoError::Canonicalize { source })?;
    for dir in canonical.ancestors() {
        match fs::symlink_metadata(dir.join(".git")) {
            // Any entry named `.git` — directory, file, symlink — is
            // a root; the walked directory IS the repository root.
            Ok(_) => return Ok(dir.to_path_buf()),
            // No `.git` at this level: climb one level up.
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            // The probe was refused, not answered: the OS refusal
            // rides out typed, the walk never pretends absence.
            Err(source) => {
                return Err(NotARepoError::NotReadable {
                    path: dir.to_string_lossy().into_owned(),
                    source,
                });
            }
        }
    }
    // Every level up to and including the filesystem root was probed
    // and none has a `.git` entry: not inside a repository.
    Err(NotARepoError::NoGitEntry {
        path: canonical.to_string_lossy().into_owned(),
    })
}

/// Why [`find_git_root`] said no.
///
/// The type is named for the §10 code the add-project flow raises on
/// the negative lookup — `E_NOT_A_REPO` ("A folder without `.git` is
/// being added"; the user picks the repository root and retries) — but
/// the variants keep the failure modes distinguishable, because a
/// refused probe is not a missing repository: `NoGitEntry` is the
/// variant that maps to `E_NOT_A_REPO`, while the io-backed variants
/// carry the exact OS failure for §10's log facts (what the command
/// layer maps them to is its call; D-062's posture keeps `AppError`
/// out of this crate). Hand-rolled impls, not `thiserror`: the crate's
/// §5.2 dependency list has no `thiserror` (the DECISIONS.md D-017
/// store precedent).
#[derive(Debug)]
pub enum NotARepoError {
    /// The input path could not be canonicalized: the folder does not
    /// exist (moved, deleted, unmounted — D-09's situation), access is
    /// denied on the way to it, a link loop cannot be followed. The
    /// typed canonicalization failure, carried whole.
    Canonicalize {
        /// The canonicalization refusal (input path text + OS error).
        source: CanonicalizeError,
    },
    /// A `.git` probe on a walked directory was refused by the OS:
    /// denied search on the folder (or, in the window between
    /// canonicalization and a later probe, on one of its ancestors),
    /// the canonicalized input being a plain file (ENOTDIR — this
    /// classification is pinned on Unix; other platforms may classify
    /// a non-directory component differently, and a plain-file input
    /// is outside this function's contract: the folder picker supplies
    /// folders), a drive dropped mid-walk. The refused path and the
    /// exact OS failure are the §10 log facts.
    NotReadable {
        /// The walked directory whose `.git` probe was refused.
        path: String,
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The folder and every ancestor up to the filesystem root were
    /// probed and none has a `.git` entry: the folder is not inside a
    /// repository — the `E_NOT_A_REPO` situation. No OS call failed,
    /// so there is no io error to carry.
    NoGitEntry {
        /// The canonicalized folder the walk started from (lossy
        /// form, the same text the log line shows).
        path: String,
    },
}

impl NotARepoError {
    /// The path text this failure is about: the canonicalization
    /// input, the walked directory whose probe was refused, or the
    /// canonicalized folder whose ascent found no repository (lossy
    /// form, the same text the §10 log line shows).
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            NotARepoError::Canonicalize { source } => source.path(),
            NotARepoError::NotReadable { path, .. } | NotARepoError::NoGitEntry { path } => path,
        }
    }

    /// The I/O error behind this failure, when an OS call produced it;
    /// `None` for the negative lookup, which is a fact about the tree
    /// rather than an OS failure.
    #[must_use]
    pub fn io_error(&self) -> Option<&io::Error> {
        match self {
            NotARepoError::Canonicalize { source } => Some(source.io_error()),
            NotARepoError::NotReadable { source, .. } => Some(source),
            NotARepoError::NoGitEntry { .. } => None,
        }
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`) — §10
    /// names it a first-class fact of every error log line. `None`
    /// where no OS call failed.
    #[must_use]
    pub fn os_code(&self) -> Option<i32> {
        self.io_error().and_then(io::Error::raw_os_error)
    }
}

impl fmt::Display for NotARepoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotARepoError::Canonicalize { source } => {
                write!(f, "cannot look up the git repository: {source}")
            }
            NotARepoError::NotReadable { path, source } => {
                write!(f, "cannot inspect {path} for a .git entry: {source}")
            }
            NotARepoError::NoGitEntry { path } => {
                write!(f, "no .git entry in {path} or any of its ancestors")
            }
        }
    }
}

impl std::error::Error for NotARepoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NotARepoError::Canonicalize { source } => Some(source),
            NotARepoError::NotReadable { source, .. } => Some(source),
            NotARepoError::NoGitEntry { .. } => None,
        }
    }
}

// ---- 9.4 preflight step 2 / 9.5 step 1: the sync target path -------

/// The resolved write target of one project's sync (§9.4 preflight
/// step 2, §9.5 step 1): the path the atomic write renames over, and
/// the resolved target's file NAME the staging grammar takes.
///
/// A struct rather than a bare `PathBuf` because the name crosses a
/// second seam: the staging file is spelled `.<name>.eunomia-<token>.tmp`
/// (9.5 step 2, D-088) and `<name>` is the RESOLVED target's — a
/// `CLAUDE.md` link pointing at `AGENTS.md` stages (and is swept,
/// D-088's coordination rule to T-066) as `AGENTS.md`. Both fields
/// come from the same resolution, so the write path and the staging
/// name can never drift apart. The name is lossy text
/// (`to_string_lossy`, the module's established form — the same
/// documented U+FFFD cost posture as the `path_key`), which is what
/// the staging-name grammar (`&str`, D-088) consumes; the write path
/// itself carries the real bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetPath {
    /// The path the atomic write renames over (9.5 step 4): the plain
    /// `<root>/<name>` for an absent (creation case) or regular-file
    /// target, the symlink-resolved real path otherwise. Canonical:
    /// the root precondition (below) plus [`canonicalize`] on a link.
    path: std::path::PathBuf,
    /// The resolved target's file name, lossy text — the staging
    /// grammar's input (9.5 step 2, D-088).
    file_name: String,
}

impl TargetPath {
    /// The path the atomic write targets (9.5 steps 1 and 4). For a
    /// symlinked target this is the REAL file's path, never the link
    /// spelling — renaming over the link spelling would replace the
    /// link with a regular file, while renaming over the resolved path
    /// keeps the link intact (T-F6).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The resolved target's file name (lossy text): the name the
    /// staging file is built from and the sweep keys on (9.5 step 2,
    /// D-088's coordination rule — a `CLAUDE.md` link stages as the
    /// real file's name).
    #[must_use]
    pub fn file_name(&self) -> &str {
        &self.file_name
    }
}

/// Resolves the target path of one project's sync and checks it
/// (§9.4 preflight step 2): given the project's canonical root and the
/// target file's NAME (`target_file_name`, §5.7 — SEC-006: only a
/// value from the enum), produces the path the write will use and the
/// resolved name the staging grammar takes.
///
/// # The verdicts, in §9.4's own words
///
/// - The target is not there: `Ok` with the plain joined path — a
///   first sync CREATES the file, so absence is not an error (pinned
///   deliberately; every other probe below is about entries that
///   exist). The parent chain still resolves when it is deeper than
///   the root, so an intermediate link cannot smuggle the creation
///   path outside (see the containment bullet).
/// - The target is a regular file: that path, resolved — the root is
///   canonical and a one-component name joins canonically, so for the
///   contract shape the resolution answers the same path (one extra
///   realpath is the price of not trusting the name: a multi-component
///   name reached through an intermediate link judges by its REAL
///   location, and the write lands on the real file).
/// - The target is a symbolic link: the path is resolved and the
///   write goes to the real file — the common case `CLAUDE.md` points
///   to `AGENTS.md` (T-F6). The whole chain resolves in one OS call
///   through the module's [`canonicalize`] (the D-086 primitive): the
///   kernel follows links of any depth, and the answer is the absolute,
///   verbatim-spelled canonical form the containment check and the
///   write both want. A `read_link` composition would re-implement the
///   walk with a depth bound for no gain.
/// - The link does not end at a real file — a dangling link, or a
///   loop the OS refuses to follow (ELOOP):
///   [`TargetPathError::BrokenLink`], the `E_TARGET_BROKEN_LINK`
///   variant.
/// - The resolved real path lies outside the project root:
///   [`TargetPathError::OutsideRoot`], the `E_TARGET_OUTSIDE_ROOT`
///   variant. The check is component-wise (`Path::strip_prefix`)
///   between canonical forms — never a string prefix (`/repo2` does
///   not start with `/repo`), and the remainder must be ordinary
///   names (a `..` component is refused, not climbed; an interior
///   `.` is inert — `components()` normalizes it away and the OS
///   resolves the same inside file).
/// - The target path is a folder — as itself, or as the real file a
///   link resolves to: [`TargetPathError::IsDir`], the
///   `E_TARGET_IS_DIR` variant. The folder verdict answers BEFORE
///   containment: a link resolving to a folder outside the root
///   reports IsDir, not OutsideRoot — both are §9.4 skips, and the
///   folder fact is the nearer cause.
/// - An OS probe was REFUSED (denied access on the way to the entry,
///   a dropped drive): [`TargetPathError::NotReadable`] with the exact
///   [`io::Error`] — "could not check" is not any of §9.4's verdicts
///   (the D-090 ruling's negative-lookup/refusal split).
///
/// # The containment check's platform posture
///
/// Both sides of the check are canonical (the root by the caller's
/// D-086 posture, the link's resolution by [`canonicalize`]), so their
/// shared prefix carries the filesystem's own spelling and the
/// byte-exact component comparison answers on every platform. When the
/// byte-exact comparison fails, the comparison retries case-folded on
/// the ONE platform whose filesystem comparison folds case as
/// UNCONDITIONAL platform behavior (Windows: NTFS/FAT, the same fold
/// the module's [`PathKeyRule::Windows`] applies) — defense-in-depth
/// against a root that arrived with non-canonical case spelling; a
/// folded miss can only widen the verdict, and the byte-exact pass
/// runs first. macOS is deliberately excluded: default APFS folds, but
/// case-sensitive APFS is a supported configuration, and a folded
/// comparison there could accept a genuinely DIFFERENT directory as
/// inside the root — the false-inside direction, which is the
/// dangerous one (a write outside the registered root). Linux keeps
/// bytes (DB-030).
///
/// # Contracts the caller holds
///
/// `root` is the project root, spelled however the caller holds it —
/// it is CANONICALIZED HERE (the D-086 posture enforced, not assumed:
/// both containment sides and every returned path live under the
/// canonical form, so a root behind a symlinked ancestor — on macOS
/// every temp fixture, `/var/…` = `/private/var/…` — cannot turn a
/// legitimate target into a false OutsideRoot). `file_name` is a
/// single ordinary file-name component from [`target_file_name`]
/// (§5.7; SEC-006: names never come from free-form strings). The
/// containment check nevertheless runs on the FINAL write path
/// whatever produced it — PHYSICALLY, not lexically: every branch
/// resolves its path through canonicalize (a regular file reached
/// through an intermediate link judges by its real location; an
/// absent final component resolves its parent chain; a parent chain
/// that does not exist has nothing to link through), and the check
/// demands ordinary names in the
/// remainder: a structurally escaping name — `..` (whose textual
/// prefix would still match) or an absolute path — is refused as
/// [`TargetPathError::OutsideRoot`] rather than silently written,
/// while an interior `.` is inert (components normalize it away; the
/// OS resolves the same inside file). The real defense is §5.7's
/// enum, this makes the fs layer
/// not silently trust it.
///
/// The D-086 verbatim-stored-path ruling is about the PROJECT path's
/// identity; the resolved target is a WRITE path, not a stored
/// identity — its canonical (verbatim-spelled) form is simply what the
/// writer opens and renames over, and what keeps the containment
/// comparison honest.
///
/// # Races are honest (TOCTOU)
///
/// The probes are separate OS calls over a live filesystem; the
/// verdict is what they observed at their moment — no snapshot, no
/// pretended atomicity (the D-084 posture). The guarantee that matters
/// is structural: the write target is the RESOLVED path, so the atomic
/// write renames over the real file and never over the link spelling
/// — the link survives (T-F6). What this function does NOT guarantee:
/// the tree can change between resolution and write. The link can be
/// re-pointed (the write lands on the previously resolved file; the
/// next sync re-resolves and converges — a sync is idempotent, §10);
/// an absent target can appear as a link (the rename then replaces
/// that link with a regular file — §9.4's skip semantics tolerate the
/// flip, and the next sync re-resolves); an intermediate component of
/// a deeper name can be re-pointed after the parent chain resolved
/// (the write lands wherever the tree holds at write time; the same
/// next-sync convergence); a resolved file can vanish
/// (the write reports its own failure, 9.5 step 7).
///
/// # Errors
///
/// [`TargetPathError`], one variant per §10 `E_TARGET_*` code plus the
/// refused-probe bucket; the variant doc comments name their codes.
/// What the command layer maps the io-backed variants to is its call
/// (D-062's posture keeps `AppError` out of this crate).
///
/// # Examples
///
/// ```
/// use eunomia_fs::paths::{canonicalize, resolve_target_path};
/// use std::fs;
///
/// let dir = tempfile::tempdir().unwrap();
/// let root = dir.path().join("repo");
/// fs::create_dir(&root).unwrap();
/// fs::write(root.join("AGENTS.md"), b"real file").unwrap();
///
/// // An absent target is the plain joined path (of the canonical root):
/// // a first sync creates it.
/// let created = resolve_target_path(&root, "CLAUDE.md").unwrap();
/// assert_eq!(created.path(), canonicalize(&root).unwrap().join("CLAUDE.md"));
/// assert_eq!(created.file_name(), "CLAUDE.md");
///
/// // The common case (T-F6): the link resolves, the write goes to the
/// // real file, and the resolved name is the real file's name.
/// #[cfg(unix)]
/// {
///     std::os::unix::fs::symlink("AGENTS.md", root.join("CLAUDE.md")).unwrap();
///     let target = resolve_target_path(&root, "CLAUDE.md").unwrap();
///     assert_eq!(
///         target.path(),
///         canonicalize(&root).unwrap().join("AGENTS.md")
///     );
///     assert_eq!(target.file_name(), "AGENTS.md");
/// }
/// ```
pub fn resolve_target_path(root: &Path, file_name: &str) -> Result<TargetPath, TargetPathError> {
    // The containment comparison must be canonical on BOTH sides, and
    // the returned paths must live under one spelling: the root
    // resolves through the module's canonicalize — the D-086 posture
    // ENFORCED here rather than assumed of the caller. A root behind
    // a symlinked ancestor (on macOS every temp fixture: `/var/…` is
    // the same folder as `/private/var/…`) must not turn a legitimate
    // target into a false OutsideRoot. A vanished or unresolvable root
    // is a refusal, not a verdict: the availability check (§9.4 step
    // 1) owns that situation in the pipeline, and this is the honest
    // answer if the tree moved mid-run.
    let root = canonicalize(root).map_err(|error| TargetPathError::NotReadable {
        path: error.path().to_owned(),
        source: error.into_io_error(),
    })?;
    let candidate = root.join(file_name);
    // Probe WITHOUT following a final symlink: the verdict "is the
    // target a link" is §9.4's fork, and a plain folder must answer
    // IsDir before any resolution is attempted.
    let write_path = match fs::symlink_metadata(&candidate) {
        // Not there: a first sync creates it — but INTERMEDIATE
        // components may exist and may be links, so the parent chain
        // resolves through canonicalize and containment judges the
        // real location (see [`resolve_absent`]).
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            resolve_absent(&root, &candidate)?
        }
        // The probe was refused, not answered: ride out typed.
        Err(source) => {
            return Err(TargetPathError::NotReadable {
                path: candidate.to_string_lossy().into_owned(),
                source,
            });
        }
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                resolve_link(&candidate)?
            } else if file_type.is_dir() {
                return Err(TargetPathError::IsDir {
                    path: candidate.to_string_lossy().into_owned(),
                });
            } else {
                // A regular file (or any non-directory, non-link
                // entry) — possibly reached THROUGH intermediate
                // links (the probe followed them): resolve the whole
                // path so containment judges the real location and
                // the write lands on the real file. For a contract
                // name (one component under the canonical root) this
                // is the same path spelled by the join — one extra
                // realpath is the price of not trusting the name.
                // The atomic rename replaces it in place.
                canonicalize(&candidate).map_err(|error| TargetPathError::NotReadable {
                    path: error.path().to_owned(),
                    source: error.into_io_error(),
                })?
            }
        }
    };
    // Containment on the FINAL write path, whatever produced it (see
    // "Contracts the caller holds" for why the plain cases run it too).
    if !inside_root(&root, &write_path) {
        return Err(TargetPathError::OutsideRoot {
            root: root.to_string_lossy().into_owned(),
            path: write_path.to_string_lossy().into_owned(),
        });
    }
    let resolved_name = match write_path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        // A canonical path with no final component is a filesystem
        // root — a folder, so §9.4's verdict is IsDir, not a panic.
        // Unreachable for contract inputs (the folder verdicts above
        // already refused every directory); kept so the function
        // stays total.
        None => {
            return Err(TargetPathError::IsDir {
                path: write_path.to_string_lossy().into_owned(),
            });
        }
    };
    Ok(TargetPath {
        path: write_path,
        file_name: resolved_name,
    })
}

/// Resolves the write path for an ABSENT final component (the
/// creation case): the parent chain resolves through canonicalize so
/// containment judges the real location an intermediate link would
/// lead to, and the final component is carried verbatim — it is the
/// name the write creates.
///
/// The root itself is already canonical, so a one-component name (the
/// contract shape, §5.7) takes the lexical join with no extra syscall.
/// A parent chain that does not exist has nothing to link through —
/// the lexical join is the truth there (a future creation is plain,
/// or the write fails on the missing folder); any other refusal to
/// resolve the parent is [`TargetPathError::NotReadable`].
fn resolve_absent(root: &Path, candidate: &Path) -> Result<std::path::PathBuf, TargetPathError> {
    // A parentless candidate is a bare relative name or a filesystem
    // root — both impossible here (the join runs under a canonical
    // absolute root, and an absent root-like path cannot be probed
    // absent); the lexical candidate keeps the function total.
    let Some(parent) = candidate.parent() else {
        return Ok(candidate.to_path_buf());
    };
    if parent == root {
        // One component under the canonical root: the lexical join IS
        // the real path.
        return Ok(candidate.to_path_buf());
    }
    match canonicalize(parent) {
        Ok(real_parent) => match candidate.file_name() {
            Some(final_component) => Ok(real_parent.join(final_component)),
            // No final component: nothing to rejoin (unreachable for
            // a join under a canonical root); the lexical candidate
            // keeps the function total.
            None => Ok(candidate.to_path_buf()),
        },
        Err(error) => {
            let source = error.into_io_error();
            if source.kind() == io::ErrorKind::NotFound {
                // The parent chain does not exist either: no link can
                // hide there, the lexical join is the truth.
                Ok(candidate.to_path_buf())
            } else {
                Err(TargetPathError::NotReadable {
                    path: candidate.to_string_lossy().into_owned(),
                    source,
                })
            }
        }
    }
}

/// Resolves a symlinked target through its whole chain to the real
/// file (§9.4 step 2). The whole-chain resolution is the module's
/// [`canonicalize`]; a chain that does not end at a real file is
/// [`TargetPathError::BrokenLink`] (dangling, or a loop the OS refuses
/// to follow), any other refusal is [`TargetPathError::NotReadable`],/// and a chain ending at a directory is [`TargetPathError::IsDir`] —
/// the write cannot go into a folder.
fn resolve_link(candidate: &Path) -> Result<std::path::PathBuf, TargetPathError> {
    let real = match canonicalize(candidate) {
        Ok(real) => real,
        Err(error) => {
            let source = error.into_io_error();
            if source.kind() == io::ErrorKind::NotFound || is_filesystem_loop(source.raw_os_error())
            {
                return Err(TargetPathError::BrokenLink {
                    path: candidate.to_string_lossy().into_owned(),
                    source,
                });
            }
            return Err(TargetPathError::NotReadable {
                path: candidate.to_string_lossy().into_owned(),
                source,
            });
        }
    };
    // The chain ended at something: classify the FINAL file (a link
    // resolving to a directory is the folder verdict, §9.4).
    match fs::metadata(&real) {
        Ok(metadata) if metadata.is_dir() => Err(TargetPathError::IsDir {
            path: real.to_string_lossy().into_owned(),
        }),
        // The file vanished between the resolution and this stat: the
        // link does not (any longer) resolve to a real file — broken.
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            Err(TargetPathError::BrokenLink {
                path: candidate.to_string_lossy().into_owned(),
                source,
            })
        }
        Err(source) => Err(TargetPathError::NotReadable {
            path: real.to_string_lossy().into_owned(),
            source,
        }),
        Ok(_) => Ok(real),
    }
}

/// Whether an OS refusal means the link chain cannot be followed
/// (ELOOP — a loop, or a chain past the OS's depth cap): the
/// resolution can never end at a real file, so the verdict is broken,
/// like a dangling link. The constant is spelled per platform (errno
/// values are platform-defined; Linux carries ELOOP at 40, macOS at
/// 62 — verified against xnu's `bsd/sys/errno.h`; 32 there is EPIPE);
/// where the platform's spelling is unknown here the refusal
/// classifies as the coarse [`TargetPathError::NotReadable`] bucket —
/// the exact OS code still rides in the source for the §10 log line.
fn is_filesystem_loop(raw_os_error: Option<i32>) -> bool {
    match raw_os_error {
        Some(code) if cfg!(target_os = "linux") => code == 40, // ELOOP
        Some(code) if cfg!(target_os = "macos") => code == 62, // ELOOP
        _ => false,
    }
}

/// Whether `candidate` lies inside `root`, compared component-wise
/// between the two canonical forms; see [`resolve_target_path`] for
/// the platform posture of the case-fold retry.
fn inside_root(root: &Path, candidate: &Path) -> bool {
    inside_root_components(root, candidate, cfg!(target_os = "windows"))
}

/// [`inside_root`] with the fold flavor explicit — the pure,
/// parameterized core (the `path_key_with_rule` idiom): the platform
/// wrapper selects, and every flavor is testable on every OS. The
/// folded pass reuses the Windows key rule per component (NFC →
/// simple fold → NFC), which is exactly the fold NTFS `$UpCase`
/// applies (DB-030's correspondence).
fn inside_root_components(root: &Path, candidate: &Path, fold_case: bool) -> bool {
    // The remainder after the root prefix must be ordinary NAMES: a
    // ParentDir left over (`<root>/../x`) climbs out textually while
    // the textual prefix still matches — refused, because the name
    // parameter is contracted to be a plain component (§5.7) and
    // nothing is written on the strength of a textual prefix alone.
    // An interior CurDir never reaches this test: components()
    // normalizes it away, and it names the same inside file.
    if let Ok(remainder) = candidate.strip_prefix(root)
        && remainder
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return true;
    }
    if !fold_case {
        return false;
    }
    let folded = |path: &Path| -> Vec<(String, bool)> {
        path.components()
            .map(|component| {
                let plain = matches!(component, std::path::Component::Normal(_));
                let text = component.as_os_str().to_string_lossy();
                (path_key_with_rule(&text, PathKeyRule::Windows), plain)
            })
            .collect()
    };
    let (root_parts, candidate_parts) = (folded(root), folded(candidate));
    root_parts.len() <= candidate_parts.len()
        && root_parts
            .iter()
            .zip(&candidate_parts)
            .all(|(root_part, candidate_part)| root_part == candidate_part)
        && candidate_parts[root_parts.len()..]
            .iter()
            .all(|(_, plain)| *plain)
}

/// Why [`resolve_target_path`] refused the target: one variant per
/// §10 code of the row "The target is a folder, a dangling link, or
/// outside the project root", named for its `E_TARGET_*` code, plus
/// the refused-probe bucket. `BrokenLink` and `NotReadable` carry the
/// OS failure (its `raw_os_error` and `ErrorKind` are §10's log
/// facts); `OutsideRoot` and `IsDir` are facts about the tree no OS
/// call failed for — their `os_code()` is `None`, the D-090
/// negative-lookup/refusal split. Hand-rolled impls, not `thiserror`:
/// the crate's §5.2 dependency list has none (the DECISIONS.md D-017
/// store precedent).
#[derive(Debug)]
pub enum TargetPathError {
    /// The target is a symlink whose chain does not end at a real
    /// file: dangling (`NotFound`), or a loop the OS refuses to
    /// follow (ELOOP, where the platform's spelling is known) — the
    /// `E_TARGET_BROKEN_LINK` variant. The
    /// path is the link spelling the probes were asked about; the OS
    /// error distinguishes the two shapes.
    BrokenLink {
        /// The link spelling whose chain does not resolve (lossy
        /// form, the same text the log line shows).
        path: String,
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The resolved real path lies outside the project root — the
    /// `E_TARGET_OUTSIDE_ROOT` variant. No OS call failed, so there is
    /// no io error to carry; both paths are the §10 log facts.
    OutsideRoot {
        /// The project root the containment was checked against
        /// (lossy form).
        root: String,
        /// The resolved real path that fell outside it (lossy form).
        path: String,
    },
    /// The target path is a folder — as itself, or as the real file a
    /// link resolves to — the `E_TARGET_IS_DIR` variant. No OS call
    /// failed, so there is no io error to carry.
    IsDir {
        /// The folder path (lossy form): the joined spelling for a
        /// plain folder, the resolved path for a link resolving to
        /// one.
        path: String,
    },
    /// An OS probe on the way to a verdict was refused: denied access
    /// on the target or an ancestor, a drive dropped mid-check, a
    /// resolution failure that is neither dangling nor a loop. The
    /// module's established coarse access bucket (D-084, D-090) with
    /// the exact [`io::Error`] riding along; what the command layer
    /// maps it to is its call.
    NotReadable {
        /// The probed path whose inspection was refused (lossy form).
        path: String,
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
}

impl TargetPathError {
    /// The path text this failure is about: the link spelling or
    /// joined target for the probe-backed variants, the resolved real
    /// path for `OutsideRoot` and `IsDir` (lossy form, the same text
    /// the §10 log line shows).
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            TargetPathError::BrokenLink { path, .. }
            | TargetPathError::OutsideRoot { path, .. }
            | TargetPathError::IsDir { path }
            | TargetPathError::NotReadable { path, .. } => path,
        }
    }

    /// The I/O error behind this failure, when an OS call produced it;
    /// `None` for the tree-fact verdicts (`OutsideRoot`, `IsDir`).
    #[must_use]
    pub fn io_error(&self) -> Option<&io::Error> {
        match self {
            TargetPathError::BrokenLink { source, .. }
            | TargetPathError::NotReadable { source, .. } => Some(source),
            TargetPathError::OutsideRoot { .. } | TargetPathError::IsDir { .. } => None,
        }
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`) — §10
    /// names it a first-class fact of every error log line. `None`
    /// where no OS call failed.
    #[must_use]
    pub fn os_code(&self) -> Option<i32> {
        self.io_error().and_then(io::Error::raw_os_error)
    }
}

impl fmt::Display for TargetPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetPathError::BrokenLink { path, source } => {
                write!(
                    f,
                    "target path {path} does not resolve to a real file: {source}"
                )
            }
            TargetPathError::OutsideRoot { root, path } => {
                write!(
                    f,
                    "target path {path} resolves outside the project root {root}"
                )
            }
            TargetPathError::IsDir { path } => {
                write!(f, "target path {path} is a directory")
            }
            TargetPathError::NotReadable { path, source } => {
                write!(f, "target path {path} cannot be inspected: {source}")
            }
        }
    }
}

impl std::error::Error for TargetPathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TargetPathError::BrokenLink { source, .. }
            | TargetPathError::NotReadable { source, .. } => {
                Some(source as &(dyn std::error::Error + 'static))
            }
            TargetPathError::OutsideRoot { .. } | TargetPathError::IsDir { .. } => None,
        }
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

    // ---- PRJ-003 / D-08: the Git repository root lookup ----------------

    #[test]
    fn the_chosen_folder_that_is_the_root_is_returned_canonicalized() {
        // The picked folder IS the repository root: the answer is the
        // canonical form of the input (the path the app stores,
        // DB-030) — the root itself, not its parent.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        let canonical = canonicalize(&repo).expect("the fixture repository must canonicalize");

        let root = find_git_root(&repo).expect("a folder holding a .git entry is itself the root");
        assert_eq!(
            root, canonical,
            "the answer must be the canonical picked folder itself, not its parent"
        );
        assert!(root.is_absolute(), "a canonical root is absolute: {root:?}");
    }

    #[test]
    fn a_subfolder_two_levels_deep_finds_the_same_root() {
        // PRJ-003's subfolder case: the app offers the repository root
        // for a folder picked anywhere inside the repository.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        let deep = repo.join("level-one").join("level-two");
        fs::create_dir_all(&deep).expect("creating the deep subfolder must work");

        let root =
            find_git_root(&deep).expect("a subfolder of a repository must resolve to the root");
        assert_eq!(
            root,
            canonicalize(&repo).expect("the fixture repository must canonicalize"),
            "the walk must stop at the nearest .git above the subfolder"
        );
    }

    #[test]
    fn a_folder_outside_any_repository_is_a_typed_no_git_entry() {
        // The non-Git case of PRJ-001/003: a folder with no `.git`
        // anywhere up the tree is rejected — typed, never a panic. The
        // temp tree has no `.git`; its REAL ancestors (up to the
        // filesystem root) must not have one either, checked with
        // plain fs probes independent of the function under test, so
        // the fixture cannot pass vacuously or fail on a repository
        // above the temp dir.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let plain = dir.path().join("plain").join("deeper");
        fs::create_dir_all(&plain).expect("creating the plain tree must work");
        // Canonicalize FIRST and guard over the CANONICAL ancestors:
        // the walk probes the ancestors of the canonical fixture path
        // (which can differ from the spelled temp path where the temp
        // dir sits behind a symlink), so the guard must inspect the
        // same set — skip(1) drops the fixture itself.
        let canonical = canonicalize(&plain).expect("the fixture folder must canonicalize");
        for ancestor in canonical.ancestors().skip(1) {
            if fs::symlink_metadata(ancestor.join(".git")).is_ok() {
                eprintln!(
                    "skipping a_folder_outside_any_repository_is_a_typed_no_git_entry: \
                     a .git entry exists above the temp tree ({ancestor:?}), the \
                     outside-any-repository fixture cannot be isolated here"
                );
                return;
            }
        }

        let canonical_text = canonical.to_string_lossy().into_owned();
        let err = find_git_root(&plain).expect_err(
            "a folder with no .git up to the filesystem root must be rejected \
             (the E_NOT_A_REPO situation)",
        );
        assert!(
            matches!(&err, NotARepoError::NoGitEntry { .. }),
            "the negative lookup must be NoGitEntry, got: {err:?}"
        );
        assert_eq!(
            err.path(),
            canonical_text,
            "the error names the canonicalized folder the walk started from"
        );
        assert_eq!(
            err.os_code(),
            None,
            "no OS call failed on a negative lookup: a fact about the tree, not a refusal"
        );
        assert!(
            err.io_error().is_none(),
            "there is no io error behind a negative lookup"
        );
        assert!(
            err.source().is_none(),
            "the source chain is empty for a negative lookup"
        );
    }

    #[test]
    fn a_git_file_terminates_the_walk_worktree_form() {
        // D-06's other form: `.git` is a FILE holding a `gitdir:`
        // pointer (a linked worktree). ALG-052 skips such folders in
        // the SCAN ("submodule or worktree"), but the add-by-hand flow
        // accepts them (§6.9: "folder or file"; D-06: "you can add it
        // by hand") — so the walk-up must terminate there, answering
        // with the worktree's own root, not following the pointer to
        // the main repository.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let worktree = dir.path().join("worktree");
        fs::create_dir(&worktree).expect("creating the fixture worktree must work");
        fs::write(worktree.join(".git"), "gitdir: ../main/.git\n")
            .expect("writing the gitdir pointer must work");
        let deep = worktree.join("nested");
        fs::create_dir(&deep).expect("creating the nested folder must work");

        let root = find_git_root(&deep)
            .expect("a .git FILE must terminate the walk exactly like a .git directory");
        assert_eq!(
            root,
            canonicalize(&worktree).expect("the fixture worktree must canonicalize"),
            "the worktree folder itself is the root"
        );
    }

    #[test]
    fn the_nearest_git_root_wins_over_an_outer_nested_repository() {
        // D-06: nested repositories are registered as separate
        // projects — the walk stops at the FIRST .git bottom-up, so a
        // folder inside the inner repository resolves to the inner
        // root, never to the outer one.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let outer = repo_with_git_entry(dir.path(), "outer");
        let inner = repo_with_git_entry(&outer, "inner");
        let inside_inner = inner.join("sub");
        fs::create_dir(&inside_inner).expect("creating the fixture folder must work");

        let root = find_git_root(&inside_inner).expect("the nearest root must be found");
        assert_eq!(
            root,
            canonicalize(&inner).expect("the inner repository must canonicalize"),
            "the nearest root wins"
        );
        assert_ne!(
            root,
            canonicalize(&outer).expect("the outer repository must canonicalize"),
            "the outer repository must not win over the inner one"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_subfolder_resolves_to_the_real_root_path() {
        // Canonicalize-first (D-086): the alias spelling never appears
        // in the answer — the root is the canonical path of the real
        // repository, the form the app stores (DB-030).
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        fs::create_dir(repo.join("sub")).expect("creating the fixture subfolder must work");
        let alias = dir.path().join("alias");
        if !symlink_or_report_untestable(&repo, &alias) {
            return;
        }
        let deep_alias = alias.join("sub");

        let root = find_git_root(&deep_alias)
            .expect("a symlinked subfolder resolves to the repository through its target");
        assert_eq!(
            root,
            canonicalize(&repo).expect("the real repository must canonicalize"),
            "the answer is the real root, not the alias spelling"
        );
        assert_ne!(root, alias, "the alias spelling is resolved away");
    }

    #[cfg(unix)]
    #[test]
    fn a_git_symlink_terminates_the_walk_like_any_git_entry() {
        // The D-084 ruling carried to the walk: "an entry named .git"
        // counts — presence, not validity. The probe is
        // symlink_metadata, which does not follow the final symlink,
        // so even a DANGLING .git symlink terminates the walk; whether
        // the pointer leads to a healthy gitdir is git's business, and
        // the scan's form distinction (ALG-052) is not this walk's.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("linked");
        fs::create_dir(&repo).expect("creating the fixture repository must work");
        std::os::unix::fs::symlink("/nowhere/gitdir", repo.join(".git"))
            .expect("creating the .git symlink must work");

        let root =
            find_git_root(&repo).expect("a .git entry that is a symlink terminates the walk");
        assert_eq!(
            root,
            canonicalize(&repo).expect("the fixture repository must canonicalize"),
            "the folder holding the .git symlink is the root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_denied_git_probe_is_not_readable_not_no_git_entry() {
        use std::os::unix::fs::PermissionsExt;

        // The walked folder is canonicalizable but not searchable:
        // mode 0o600 (no x) — canonicalizing the FINAL component needs
        // search permission only on its parent, while the
        // `<folder>/.git` probe needs search ON the folder (verified
        // on this platform: realpath succeeds, the probe answers
        // EACCES). The walk must report the refusal as a typed
        // NotReadable carrying EACCES, never fold it into "no
        // repository": between canonicalization and any probe a
        // permission can change (the same window covers an ancestor
        // denied mid-walk), and "could not check" is not "checked and
        // absent".
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("locked");
        fs::create_dir(&repo).expect("creating the fixture folder must work");
        if running_as_root_is_untestable(&repo) {
            eprintln!(
                "skipping a_denied_git_probe_is_not_readable_not_no_git_entry: \
                 running as root — mode bits are not enforced, the \
                 unsearchable-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&repo, fs::Permissions::from_mode(0o600))
            .expect("removing the search bit must work");
        // Prove the failure below comes from the refused probe and not
        // from an earlier stage: canonicalization must still succeed
        // while the search bit is gone. Captured while the fixture is
        // locked but asserted only after the restore, so even a
        // precondition panic cannot strand an unremovable fixture.
        let canonicalization_still_ok = canonicalize(&repo).is_ok();
        let result = find_git_root(&repo);
        fs::set_permissions(&repo, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");
        assert!(
            canonicalization_still_ok,
            "precondition: a mode-0o600 folder must still canonicalize"
        );

        let err = result.expect_err("a denied .git probe must be an error");
        assert!(
            matches!(&err, NotARepoError::NotReadable { .. }),
            "a refused probe must be NotReadable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(13),
            "EACCES is the OS code the §10 log line carries for denied search"
        );
        let canonical_text = canonicalize(&repo)
            .expect("the restored folder must canonicalize")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            err.path(),
            canonical_text,
            "the error names the refused walked directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_plain_file_input_surfaces_enotdir_as_not_readable() {
        // canonicalize accepts any existing path, so a plain file
        // reaches the walk: probing `<file>/.git` answers ENOTDIR —
        // classified NotReadable (the path cannot be a folder), with
        // the errno riding in the typed error, never NoGitEntry. The
        // classification is pinned on Unix; on other platforms the OS
        // may classify a non-directory component differently — a
        // plain-file input is outside the function's contract (the
        // folder picker supplies folders), documented on the variant.
        // Runs regardless of privileges, unlike the permission test
        // above.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let file = dir.path().join("plain.txt");
        fs::write(&file, b"not a folder").expect("writing the fixture file must work");

        let err = find_git_root(&file)
            .expect_err("a plain file is not a folder that could sit inside a repository");
        assert!(
            matches!(&err, NotARepoError::NotReadable { .. }),
            "ENOTDIR must be NotReadable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(20),
            "ENOTDIR = 20 must ride into the typed error"
        );
    }

    #[test]
    fn a_missing_input_is_the_canonicalize_failure() {
        // A folder that does not exist cannot even be canonicalized:
        // the module's existing typed failure (D-086), carried whole —
        // path text and OS code for the §10 log line.
        let missing = Path::new("/definitely/not/here");
        let err = find_git_root(missing)
            .expect_err("a missing folder must fail canonicalization before any walk");
        assert!(
            matches!(&err, NotARepoError::Canonicalize { .. }),
            "a missing input must surface the canonicalize failure, got: {err:?}"
        );
        assert_eq!(
            err.path(),
            "/definitely/not/here",
            "the error names the input the walk was asked about"
        );
        #[cfg(unix)]
        assert_eq!(
            err.os_code(),
            Some(2),
            "ENOENT is the code the §10 log line carries for a missing folder"
        );
        assert!(
            err.source().is_some(),
            "the canonicalize failure rides in the source chain"
        );
    }

    #[test]
    fn the_root_lookup_error_contract_follows_the_module_style() {
        // The command layer branches on the variants and logs the
        // Display text; these assertions pin the wording (lowercase,
        // no trailing punctuation — the house style), the path()
        // accessor per variant, the io-backed/os-backed split and the
        // source() chain — without pinning std's strerror table.
        let no_git = NotARepoError::NoGitEntry {
            path: "/tmp/plain".into(),
        };
        assert_eq!(
            no_git.to_string(),
            "no .git entry in /tmp/plain or any of its ancestors"
        );
        assert_eq!(no_git.path(), "/tmp/plain");
        assert!(
            no_git.io_error().is_none(),
            "there is no io error behind a negative lookup"
        );
        assert_eq!(no_git.os_code(), None);
        assert!(no_git.source().is_none());

        let not_readable = NotARepoError::NotReadable {
            path: "/tmp/locked".into(),
            source: io::Error::from_raw_os_error(13),
        };
        assert!(
            not_readable
                .to_string()
                .starts_with("cannot inspect /tmp/locked for a .git entry: "),
            "the NotReadable text must state the fact, then the OS text: {not_readable}"
        );
        assert_eq!(not_readable.path(), "/tmp/locked");
        assert!(not_readable.io_error().is_some());
        assert_eq!(not_readable.os_code(), Some(13));
        assert!(not_readable.source().is_some());

        let canonicalize_failure =
            canonicalize(Path::new("/definitely/not/here")).expect_err("a missing path fails");
        let wrapped = NotARepoError::Canonicalize {
            source: canonicalize_failure,
        };
        assert!(
            wrapped
                .to_string()
                .starts_with("cannot look up the git repository: "),
            "the Canonicalize text must state the fact, then the inner text: {wrapped}"
        );
        assert_eq!(wrapped.path(), "/definitely/not/here");
        assert!(wrapped.io_error().is_some());
        assert!(wrapped.os_code().is_some());
        assert!(wrapped.source().is_some());
    }

    // ---- 9.4 preflight step 2: the sync target path --------------------

    #[test]
    fn an_absent_target_is_the_plain_joined_path_the_creation_case() {
        // §9.4's implicit first case: a first sync CREATES the file, so
        // an absent target is `Ok` with the plain joined spelling —
        // and resolving changed nothing (preflight "changes nothing").
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");

        let target = resolve_target_path(&root, "AGENTS.md")
            .expect("an absent target is not an error: the write creates it");
        assert_eq!(
            target.path(),
            canonicalize(&root)
                .expect("the fixture root must canonicalize")
                .join("AGENTS.md"),
            "the write path is the plain joined spelling of the CANONICAL \
             root, verbatim — no target resolution is performed"
        );
        assert_eq!(target.file_name(), "AGENTS.md");
        assert!(
            !root.join("AGENTS.md").exists(),
            "resolution must not create anything: preflight changes nothing"
        );
    }

    #[test]
    fn a_regular_file_target_keeps_the_plain_path() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        fs::write(root.join("AGENTS.md"), b"real file")
            .expect("writing the fixture file must work");

        let target =
            resolve_target_path(&root, "AGENTS.md").expect("a regular file needs no resolution");
        assert_eq!(
            target.path(),
            canonicalize(&root)
                .expect("the fixture root must canonicalize")
                .join("AGENTS.md"),
            "a regular file is written at the joined path, unchanged"
        );
        assert_eq!(target.file_name(), "AGENTS.md");
    }

    #[cfg(unix)]
    #[test]
    fn a_claude_md_link_resolves_the_write_to_the_real_agents_md() {
        // THE card's acceptance case (T-F6): `CLAUDE.md` is a symlink
        // to `AGENTS.md` — the write goes to the real file, the
        // resolved staging name is the real file's name, and the link
        // itself is untouched (resolution only reads).
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        fs::write(root.join("AGENTS.md"), b"real file").expect("writing the real file must work");
        if !symlink_or_report_untestable(Path::new("AGENTS.md"), &root.join("CLAUDE.md")) {
            return;
        }

        let target = resolve_target_path(&root, "CLAUDE.md")
            .expect("the common case: a link to a real file inside the root");
        let real = canonicalize(&root)
            .expect("the fixture root must canonicalize")
            .join("AGENTS.md");
        assert_eq!(
            target.path(),
            real,
            "§9.4: the write goes to the real file, not the link spelling"
        );
        assert_eq!(
            target.file_name(),
            "AGENTS.md",
            "the resolved name is the real file's name (the D-088 staging rule)"
        );
        assert_ne!(
            target.path(),
            root.join("CLAUDE.md"),
            "the link spelling is resolved away"
        );
        let link =
            fs::symlink_metadata(root.join("CLAUDE.md")).expect("the link must still be there");
        assert!(
            link.file_type().is_symlink(),
            "resolution only reads: the link survives, got: {link:?}"
        );

        // The plain half of the pair resolves to itself at the same
        // real path — one write target for both spellings.
        let direct = resolve_target_path(&root, "AGENTS.md")
            .expect("a regular file target needs no resolution");
        assert_eq!(direct.path(), real);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_chain_of_depth_two_resolves_to_the_final_file() {
        // The whole chain resolves in one call — no per-level walk of
        // our own: CLAUDE.md → mid.md → AGENTS.md answers AGENTS.md.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        fs::write(root.join("AGENTS.md"), b"real file").expect("writing the real file must work");
        if !symlink_or_report_untestable(Path::new("AGENTS.md"), &root.join("mid.md")) {
            return;
        }
        if !symlink_or_report_untestable(Path::new("mid.md"), &root.join("CLAUDE.md")) {
            return;
        }

        let target = resolve_target_path(&root, "CLAUDE.md")
            .expect("a chain of depth 2 ends at the real file");
        assert_eq!(
            target.path(),
            canonicalize(&root)
                .expect("the fixture root must canonicalize")
                .join("AGENTS.md"),
            "the chain resolves past the intermediate link"
        );
        assert_eq!(target.file_name(), "AGENTS.md");
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_is_a_typed_broken_link() {
        // The card's E_TARGET_BROKEN_LINK case: the link points at a
        // file that does not exist — skipped, typed, never a panic.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        if !symlink_or_report_untestable(Path::new("nowhere.md"), &root.join("CLAUDE.md")) {
            return;
        }

        let err = resolve_target_path(&root, "CLAUDE.md")
            .expect_err("a dangling link must be refused, not written through");
        assert!(
            matches!(&err, TargetPathError::BrokenLink { .. }),
            "a dangling link must be BrokenLink, got: {err:?}"
        );
        assert_eq!(
            err.path(),
            canonicalize(&root)
                .expect("the fixture root must canonicalize")
                .join("CLAUDE.md")
                .to_string_lossy(),
            "the error names the link spelling the probes were asked about"
        );
        assert_eq!(
            err.os_code(),
            Some(2),
            "ENOENT is the OS code the §10 log line carries for a dangling link"
        );
        assert!(err.io_error().is_some());
        assert!(err.source().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_is_broken_not_unreadable() {
        // A link that follows back to itself never ends at a real
        // file: broken, like a dangling link — not "cannot inspect"
        // (the OS's ELOOP answer is about the link structure, which is
        // exactly what E_TARGET_BROKEN_LINK describes).
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        if !symlink_or_report_untestable(Path::new("loop.md"), &root.join("loop.md")) {
            return;
        }

        let err =
            resolve_target_path(&root, "loop.md").expect_err("a loop does not end at a real file");
        assert!(
            matches!(&err, TargetPathError::BrokenLink { .. }),
            "a loop must be BrokenLink, got: {err:?}"
        );
        assert!(
            err.os_code().is_some(),
            "the OS refused the resolution: the code rides along"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_resolving_outside_the_root_is_typed_outside_root() {
        // The card's E_TARGET_OUTSIDE_ROOT case, with a REAL file
        // outside the fixture root — the containment check's true
        // test: the link would resolve, and only the prefix check
        // stops the write.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        let outside = dir.path().join("outside.md");
        fs::write(&outside, b"not ours").expect("writing the outside file must work");
        if !symlink_or_report_untestable(Path::new("../outside.md"), &root.join("CLAUDE.md")) {
            return;
        }

        let err = resolve_target_path(&root, "CLAUDE.md")
            .expect_err("a real path outside the root must be refused");
        let (root_text, path_text) = match &err {
            TargetPathError::OutsideRoot { root, path } => (root.as_str(), path.as_str()),
            other => panic!("must be OutsideRoot, got: {other:?}"),
        };
        assert_eq!(
            path_text,
            canonicalize(&outside)
                .expect("the outside file must canonicalize")
                .to_string_lossy(),
            "the error names the RESOLVED real path, not the link spelling"
        );
        assert_eq!(
            root_text,
            canonicalize(&root)
                .expect("the fixture root must canonicalize")
                .to_string_lossy(),
            "the error names the canonical root the containment was checked against"
        );
        assert_eq!(
            err.os_code(),
            None,
            "containment is a fact about the tree: no OS call failed"
        );
        assert!(err.io_error().is_none());
        assert!(err.source().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_resolving_through_to_a_directory_is_the_dir_verdict() {
        // A symlink to a symlink to a folder: the chain resolves to a
        // directory, and the write cannot go into a folder — the same
        // E_TARGET_IS_DIR verdict as a plain folder target.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        fs::create_dir(root.join("sub")).expect("creating the fixture folder must work");
        if !symlink_or_report_untestable(Path::new("sub"), &root.join("alias-sub")) {
            return;
        }
        if !symlink_or_report_untestable(Path::new("alias-sub"), &root.join("CLAUDE.md")) {
            return;
        }

        let err =
            resolve_target_path(&root, "CLAUDE.md").expect_err("§9.4: the target path is a folder");
        assert!(
            matches!(&err, TargetPathError::IsDir { .. }),
            "a link resolving to a folder must be IsDir, got: {err:?}"
        );
        assert_eq!(
            err.path(),
            canonicalize(&root)
                .expect("the fixture root must canonicalize")
                .join("sub")
                .to_string_lossy(),
            "the folder verdict names the resolved path"
        );
    }

    #[test]
    fn a_plain_directory_target_is_the_dir_verdict() {
        // E_TARGET_IS_DIR without any link involved: the target path
        // itself is a folder.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        fs::create_dir(root.join("AGENTS.md")).expect("creating the fixture folder must work");

        let err =
            resolve_target_path(&root, "AGENTS.md").expect_err("§9.4: the target path is a folder");
        assert!(
            matches!(&err, TargetPathError::IsDir { .. }),
            "a plain folder target must be IsDir, got: {err:?}"
        );
        assert_eq!(
            err.path(),
            canonicalize(&root)
                .expect("the fixture root must canonicalize")
                .join("AGENTS.md")
                .to_string_lossy(),
            "the plain folder verdict names the joined spelling"
        );
        assert_eq!(
            err.os_code(),
            None,
            "the probes succeeded: a fact about the tree, no OS code"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_relative_link_spelling_resolves_through_the_link_s_folder() {
        // The link text has no leading separator: resolution is
        // relative to the LINK's folder, and a multi-component
        // relative spelling still lands inside the root.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        fs::create_dir(root.join("sub")).expect("creating the fixture folder must work");
        fs::write(root.join("sub").join("AGENTS.md"), b"real file")
            .expect("writing the real file must work");
        if !symlink_or_report_untestable(Path::new("sub/AGENTS.md"), &root.join("CLAUDE.md")) {
            return;
        }

        let target = resolve_target_path(&root, "CLAUDE.md")
            .expect("a relative spelling resolves through the link's folder");
        assert_eq!(
            target.path(),
            canonicalize(&root)
                .expect("the fixture root must canonicalize")
                .join("sub")
                .join("AGENTS.md"),
        );
        assert_eq!(target.file_name(), "AGENTS.md");
    }

    #[cfg(unix)]
    #[test]
    fn an_intermediate_symlink_cannot_smuggle_the_target_outside() {
        // The round-2 defense, both halves: a multi-component name is
        // out of contract (§5.7 names are one component), but the fs
        // layer does not lean on that — the regular-file branch
        // resolves its whole path and the absent branch resolves its
        // parent chain, so a link hiding in an INTERMEDIATE component
        // judges by the real location. The reviewer's probe, pinned:
        // `sub/AGENTS.md` with `sub → ../outside`.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).expect("creating the outside folder must work");
        fs::write(outside.join("AGENTS.md"), b"not ours")
            .expect("writing the outside file must work");
        if !symlink_or_report_untestable(Path::new("../outside"), &root.join("sub")) {
            return;
        }

        // Half one: the final component EXISTS behind the intermediate
        // link — the probe follows it, the regular-file branch
        // canonicalizes, containment sees the real outside path.
        let existing = resolve_target_path(&root, "sub/AGENTS.md")
            .expect_err("a target physically outside the root must be refused");
        assert!(
            matches!(&existing, TargetPathError::OutsideRoot { .. }),
            "an intermediate link must not accept an outside target, got: {existing:?}"
        );
        assert_eq!(
            existing.path(),
            canonicalize(&outside)
                .expect("the outside folder must canonicalize")
                .join("AGENTS.md")
                .to_string_lossy(),
            "the error names the real outside path the write would have used"
        );

        // Half two: the final component is ABSENT behind the
        // intermediate link — the parent chain resolves, containment
        // sees the real outside folder the creation would land in.
        fs::remove_file(outside.join("AGENTS.md")).expect("removing must work");
        let absent = resolve_target_path(&root, "sub/AGENTS.md")
            .expect_err("a creation behind an outside intermediate link must be refused");
        assert!(
            matches!(&absent, TargetPathError::OutsideRoot { .. }),
            "the absent branch must judge the resolved parent, got: {absent:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn containment_is_byte_exact_a_case_differing_sibling_is_outside() {
        // This platform's posture, pinned so it cannot drift: on Linux
        // `repo` and `REPO` are different folders, and a link into the
        // case-differing sibling is OUTSIDE the root — the check must
        // not fold case where the filesystem does not.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        let sibling = dir.path().join("REPO");
        fs::create_dir(&sibling).expect("creating the case-differing sibling must work");
        fs::write(sibling.join("x.md"), b"other folder")
            .expect("writing the sibling file must work");
        if !symlink_or_report_untestable(Path::new("../REPO/x.md"), &root.join("CLAUDE.md")) {
            return;
        }

        let err = resolve_target_path(&root, "CLAUDE.md")
            .expect_err("a link into a case-differing sibling folder is outside the root on Linux");
        assert!(
            matches!(&err, TargetPathError::OutsideRoot { .. }),
            "the byte-exact containment must refuse, got: {err:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn on_windows_a_case_variant_root_spelling_stays_inside() {
        // The observable the platform owes: a root spelled with
        // non-canonical case must never read as outside the root. Two
        // layers absorb it — the root canonicalizes here (the on-disk
        // spelling comes back from GetFinalPathNameByHandle), and the
        // folded containment retry would accept behind it. Creating a
        // file symlink needs SeCreateSymbolicLinkPrivilege: only that
        // capability absence skips (ERROR_PRIVILEGE_NOT_HELD = 1314);
        // anything else panics.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        fs::write(root.join("AGENTS.md"), b"real file").expect("writing the real file must work");
        if let Err(error) =
            std::os::windows::fs::symlink_file(root.join("AGENTS.md"), root.join("CLAUDE.md"))
        {
            if error.raw_os_error() == Some(1314) {
                eprintln!(
                    "skipping the Windows containment test: the runner lacks \
                     SeCreateSymbolicLinkPrivilege ({error}), the symlink \
                     fixture cannot be created here"
                );
                return;
            }
            panic!("creating the file-symlink fixture failed: {error}");
        }

        // The same folder spelled with a case-flipped name: the byte-
        // exact prefix fails, the folded retry must accept.
        let canonical = canonicalize(&root).expect("the fixture root must canonicalize");
        let variant_name = {
            let name = canonical
                .file_name()
                .expect("a canonical root has a file name")
                .to_string_lossy()
                .into_owned();
            let mut chars: Vec<char> = name.chars().collect();
            let first = chars[0].to_ascii_lowercase();
            chars[0] = if first == chars[0] {
                chars[0].to_ascii_uppercase()
            } else {
                first
            };
            chars.iter().collect::<String>()
        };
        let variant_root = canonical
            .parent()
            .expect("a canonical root has a parent")
            .join(variant_name);
        assert_ne!(
            variant_root, canonical,
            "precondition: the variant spelling must differ from the canonical one"
        );

        let target = resolve_target_path(&variant_root, "CLAUDE.md")
            .expect("a case-only root spelling drift must not read as outside the root on Windows");
        assert_eq!(
            target.path(),
            canonical.join("AGENTS.md"),
            "the write path is the canonical real file"
        );
    }

    #[test]
    fn the_containment_core_follows_the_fold_flavor() {
        // The pure, parameterized core (the `path_key_with_rule`
        // idiom): both flavors are pinned on every OS; the platform
        // wrapper selects (byte-exact here, the folded retry on
        // Windows — pinned by the cfg-gated leg above).
        let root = Path::new("/data/Repo");
        let inside = Path::new("/data/Repo/sub/x.md");
        let case_variant = Path::new("/data/repo/x.md");
        let sibling = Path::new("/data/Repo2/x.md");

        for fold_case in [false, true] {
            assert!(
                inside_root_components(root, inside, fold_case),
                "a subpath is inside, fold_case = {fold_case}"
            );
            assert!(
                inside_root_components(root, root, fold_case),
                "the root itself is inside, fold_case = {fold_case}"
            );
            assert!(
                !inside_root_components(root, sibling, fold_case),
                "a name-sharing string prefix is not inside (component-wise, \
                 fold_case = {fold_case})"
            );
            // The textual-prefix trap: `<root>/../x` strips-prefix to
            // a remainder that is NOT ordinary names — refused in both
            // flavors, nothing is written on the strength of a textual
            // prefix alone. An absolute spelling replaces the root and
            // is refused too. A `.` is different: `components()`
            // normalizes interior CurDir components away (only a
            // leading one survives), and the OS resolves it to the
            // same inside file — inert, accepted.
            assert!(
                !inside_root_components(root, Path::new("/data/Repo/../evil.md"), fold_case),
                "a ParentDir remainder climbs out: refused, fold_case = {fold_case}"
            );
            assert!(
                !inside_root_components(root, Path::new("/etc/evil.md"), fold_case),
                "an absolute spelling replaces the root: refused, \
                 fold_case = {fold_case}"
            );
            assert!(
                inside_root_components(root, Path::new("/data/Repo/./x.md"), fold_case),
                "an interior CurDir is inert — the same inside file: \
                 accepted, fold_case = {fold_case}"
            );
        }
        assert!(
            !inside_root_components(root, case_variant, false),
            "the byte-exact flavor keeps a case-different spelling outside"
        );
        assert!(
            inside_root_components(root, case_variant, true),
            "the folded flavor accepts it — the Windows retry"
        );
    }

    #[test]
    fn an_escaping_name_is_refused_as_outside_root() {
        // The uniform-containment defense, pinned end to end: the name
        // parameter is contracted to be one ordinary component (§5.7,
        // SEC-006), but a structurally escaping spelling must not be
        // silently written even if a caller ever broke that contract —
        // `..` strips-prefix TEXTUALLY (verified: the remainder is
        // `../evil.md`), so the check demands ordinary names.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");

        let parent_escape = resolve_target_path(&root, "../evil.md")
            .expect_err("a `..` name climbs out of the root: refused");
        assert!(
            matches!(&parent_escape, TargetPathError::OutsideRoot { .. }),
            "the ParentDir escape must be OutsideRoot, got: {parent_escape:?}"
        );
        assert_eq!(
            parent_escape.os_code(),
            None,
            "a containment refusal is a fact about the path, no OS code"
        );

        let absolute = resolve_target_path(&root, "/definitely/not/here.md")
            .expect_err("an absolute name replaces the root: refused");
        assert!(
            matches!(&absolute, TargetPathError::OutsideRoot { .. }),
            "the absolute escape must be OutsideRoot, got: {absolute:?}"
        );
        assert_eq!(
            absolute.path(),
            "/definitely/not/here.md",
            "the error names the path the write would have used"
        );

        let curdir = resolve_target_path(&root, "./AGENTS.md").expect(
            "an interior `.` is inert — components() normalizes it \
                    away and the OS resolves the same inside file",
        );
        assert_eq!(
            curdir.file_name(),
            "AGENTS.md",
            "the inert re-spelling resolves to the same file name"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_denied_probe_on_the_target_is_not_readable() {
        use std::os::unix::fs::PermissionsExt;

        // The probe `symlink_metadata(<root>/<name>)` needs search on
        // the root: without the search bit it answers EACCES, which is
        // a REFUSAL — not any of §9.4's three verdicts. Loud root
        // skip; mode restored before the assertions so a panic cannot
        // strand the fixture.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        if running_as_root_is_untestable(&root) {
            eprintln!(
                "skipping a_denied_probe_on_the_target_is_not_readable: \
                 running as root — mode bits are not enforced, the \
                 unsearchable-root precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&root, fs::Permissions::from_mode(0o600))
            .expect("removing the search bits must work");
        let result = resolve_target_path(&root, "AGENTS.md");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let err = result.expect_err("a refused target probe must be an error");
        assert!(
            matches!(&err, TargetPathError::NotReadable { .. }),
            "a denied probe must be NotReadable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(13),
            "EACCES is the OS code the §10 log line carries for denied access"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_verdict_is_what_the_probes_observed_not_a_snapshot() {
        // The D-084 vanished-`.git` posture, on the target: the link
        // resolves while the real file exists; remove the file and the
        // SAME link is broken. The verdict is recomputed from the
        // tree, never cached — and the resolution-to-write race is the
        // documented TOCTOU posture's observable edge.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture repository must work");
        fs::write(root.join("AGENTS.md"), b"real file").expect("writing the real file must work");
        if !symlink_or_report_untestable(Path::new("AGENTS.md"), &root.join("CLAUDE.md")) {
            return;
        }
        resolve_target_path(&root, "CLAUDE.md")
            .expect("the link resolves while the real file exists");

        fs::remove_file(root.join("AGENTS.md")).expect("removing the real file must work");
        let err = resolve_target_path(&root, "CLAUDE.md")
            .expect_err("a now-dangling link must be broken at this check");
        assert!(
            matches!(&err, TargetPathError::BrokenLink { .. }),
            "the recomputed verdict must be BrokenLink, got: {err:?}"
        );
    }

    #[test]
    fn the_target_error_contract_follows_the_module_style() {
        // The command layer branches on the variants and logs the
        // Display text; these assertions pin the wording (lowercase,
        // no trailing punctuation — the house style), the path()
        // accessor per variant, the io-backed/fact split and the
        // source() chain — without pinning std's strerror table.
        let broken = TargetPathError::BrokenLink {
            path: "/repo/CLAUDE.md".into(),
            source: io::Error::from_raw_os_error(2),
        };
        assert!(
            broken
                .to_string()
                .starts_with("target path /repo/CLAUDE.md does not resolve to a real file: "),
            "the BrokenLink text must state the fact, then the OS text: {broken}"
        );
        assert_eq!(broken.path(), "/repo/CLAUDE.md");
        assert_eq!(broken.os_code(), Some(2));
        assert!(broken.io_error().is_some());
        assert!(broken.source().is_some());

        let outside = TargetPathError::OutsideRoot {
            root: "/repo".into(),
            path: "/etc/passwd".into(),
        };
        assert_eq!(
            outside.to_string(),
            "target path /etc/passwd resolves outside the project root /repo"
        );
        assert_eq!(
            outside.path(),
            "/etc/passwd",
            "the subject of the failure is the resolved path"
        );
        assert_eq!(outside.os_code(), None);
        assert!(outside.io_error().is_none());
        assert!(outside.source().is_none());

        let is_dir = TargetPathError::IsDir {
            path: "/repo/sub".into(),
        };
        assert_eq!(is_dir.to_string(), "target path /repo/sub is a directory");
        assert_eq!(is_dir.path(), "/repo/sub");
        assert_eq!(is_dir.os_code(), None);
        assert!(is_dir.source().is_none());

        let not_readable = TargetPathError::NotReadable {
            path: "/repo/locked".into(),
            source: io::Error::from_raw_os_error(13),
        };
        assert!(
            not_readable
                .to_string()
                .starts_with("target path /repo/locked cannot be inspected: "),
            "the NotReadable text must state the fact, then the OS text: {not_readable}"
        );
        assert_eq!(not_readable.path(), "/repo/locked");
        assert_eq!(not_readable.os_code(), Some(13));
        assert!(not_readable.source().is_some());
    }
}
