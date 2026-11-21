//! Temporary Git repository roots for benchmarks and integration
//! fixtures (spec §12.2 data sets, §15.1; task T-136).
//!
//! [`TempRepos`] lays out `count` folders under one self-cleaning
//! temporary directory, each carrying a **minimal but real `.git`
//! directory** — `HEAD`, `config`, `objects/` and `refs/`, the four
//! entries `git` itself requires to call a folder a repository — so
//! the §12.2 benchmarks have the targets the sync pipeline needs:
//! folders the repository scan finds (ALG-052's `.git`-directory
//! probe), paths the preflight availability check accepts, and roots
//! a sync can write `AGENTS.md` into. Everything disappears when the
//! guard is dropped (the `tempfile::TempDir` RAII rule); the crate's
//! `tempfile` dependency was pinned from the start for exactly this
//! staging shape (see `Cargo.toml`).
//!
//! The intended composition with the store-side generator (task
//! T-136's other half, `eunomia_store::data_sets`): create the roots
//! here, then fill the database with those roots as the projects'
//! canonical paths and identity keys —
//!
//! ```text
//! let repos = TempRepos::create(500, "eunomia-bench")?;
//! // Owned lossy spellings first, borrowed afterwards: a ProjectRoot
//! // borrows, so the strings it points at must outlive the loop
//! // (`to_string_lossy`, never `to_str().unwrap_or_default()` — an
//! // odd path degrades to U+FFFD in the DISPLAY, not to an empty
//! // project path; the roots themselves are ASCII `repo-NNNN` and
//! // key losslessly, inside the D-086 addendum's domain).
//! let owned: Vec<(String, String)> = repos.roots().iter()
//!     .map(|root| (root.path.to_string_lossy().into_owned(), root.path_key.clone()))
//!     .collect();
//! let roots: Vec<ProjectRoot> = owned.iter()
//!     .map(|(path, path_key)| ProjectRoot { path, path_key })
//!     .collect();
//! generate(&mut conn, &L, &roots)?;
//! ```
//!
//! (Composed in the benchmark crate — the two library crates share no
//! dependency edge, by design: the store stays file-system-free, the
//! fs layer stays database-free.)
//!
//! This module is fixture tooling, not product flow: nothing in the
//! app registers these repositories, and nothing here writes target
//! files — the sync benchmarks do that through the real pipeline.

use std::fs;
use std::io;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::paths::{canonicalize, path_key};

/// Maps the fixture's canonicalization refusal onto the plain
/// [`io::Error`] the fixture's own signature returns: the OS kind for
/// an OS refusal, `Other` for the NotUtf8 refusal (unreachable here —
/// the labels are validated ASCII — but mapped honestly, never
/// `unwrap`ped).
fn fixture_canonicalize_error(error: crate::paths::CanonicalizeError) -> io::Error {
    let kind = error
        .io_error()
        .map_or(io::ErrorKind::Other, io::Error::kind);
    io::Error::new(kind, error.to_string())
}

/// One created repository root: its canonical path (DB-030, the
/// spelling a registered project stores) and the matching identity
/// key — the exact pair the store-side generator's `ProjectRoot`
/// input wants, so benchmarks pass the values through without
/// re-deriving them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRoot {
    /// The canonical repository path.
    pub path: PathBuf,
    /// The DB-030/DB-031 identity key of that path.
    pub path_key: String,
}

/// A set of temporary Git repository roots under one temporary base
/// directory, removed with the guard (drop = cleanup on the normal
/// unwinding paths, panics included; a hard abort — `panic =
/// "abort"` in release — leaves the tree to the OS's temp sweeping,
/// the same as any aborted temp user).
///
/// The roots live at `<base>/<label>/repo-<index:04>`; `label` names
/// the group in the temp directory and must be a single component: a
/// non-empty name without path separators and without `..` (a slash
/// would nest the group, `..` would escape the guard's base and
/// survive the cleanup — rejected up front with
/// [`io::ErrorKind::InvalidInput`]).
#[derive(Debug)]
pub struct TempRepos {
    /// Keeps the base directory alive for `'self` and removes it on
    /// drop — the whole point of the guard. Underscore-named: holding
    /// it is the effect, nothing reads it back (the group directory
    /// below is the addressable part).
    _base: TempDir,
    /// The group directory the roots sit in (kept for scan roots).
    group_dir: PathBuf,
    /// The roots, in creation (index) order.
    roots: Vec<RepoRoot>,
}

impl TempRepos {
    /// Creates `count` repository roots at
    /// `<tempdir>/<label>/repo-<index:04>`, each with the minimal
    /// valid `.git` layout, and returns them with canonical paths and
    /// identity keys. `count = 0` creates the group directory and no
    /// roots — a valid empty fixture.
    ///
    /// # Errors
    ///
    /// [`io::Error`] with [`io::ErrorKind::InvalidInput`] for a label
    /// that is not a single safe component; otherwise an [`io::Error`]
    /// when the temporary directory or any repository entry cannot be
    /// created, or a created root cannot be canonicalized — a full
    /// disk or a hostile TMPDIR; nothing is left behind in either
    /// case (the guard drops with the error).
    pub fn create(count: usize, label: &str) -> io::Result<Self> {
        if label.is_empty()
            || label.contains(['/', '\\'])
            || label.split(['/', '\\']).any(|component| component == "..")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("the label {label:?} must be one non-empty path component without '..'"),
            ));
        }
        let base = TempDir::new()?;
        let group = base.path().join(label);
        fs::create_dir_all(&group)?;
        // The canonical spelling (on macOS the system temp directory
        // sits behind a `/var` → `/private/var` symlink): the roots
        // are canonical, so the group directory they are compared
        // against and the scan walks must be too. The fixture's labels
        // are validated ASCII components, so the identity seam's
        // NotUtf8 refusal (the D-086 addendum) is unreachable here.
        let group_dir = canonicalize(&group).map_err(fixture_canonicalize_error)?;
        let mut repos = Self {
            _base: base,
            group_dir,
            roots: Vec::with_capacity(count),
        };
        for index in 0..count {
            let repo = repos.group_dir.join(format!("repo-{index:04}"));
            write_minimal_git(&repo)?;
            let canonical = canonicalize(&repo).map_err(fixture_canonicalize_error)?;
            // The key of the already-canonical form — the same value
            // `canonical_path_key` would return, without a second
            // canonicalization pass.
            let path_key =
                path_key(&canonical).map_err(|error| io::Error::other(error.to_string()))?;
            repos.roots.push(RepoRoot {
                path: canonical,
                path_key,
            });
        }
        Ok(repos)
    }

    /// The created roots, in creation order: canonical paths and
    /// identity keys, unique by construction (each root is its own
    /// canonical folder).
    #[must_use = "the fixture exists to be read; dropping it deletes the repositories"]
    pub fn roots(&self) -> &[RepoRoot] {
        &self.roots
    }

    /// The group directory holding every root — the canonical parent
    /// a repository scan (ALG-050) would walk to find them all.
    #[must_use = "the fixture exists to be read; dropping it deletes the repositories"]
    pub fn group_dir(&self) -> &std::path::Path {
        &self.group_dir
    }
}

/// Writes the minimal valid `.git` directory into `repo` — the four
/// entries `git` requires (`HEAD`, `objects/`, `refs/`, plus a
/// `config` for good measure): enough for ALG-052's directory probe,
/// `git rev-parse --git-dir`, and every future tooling a benchmark
/// might spawn. No commit history, no working tree files: §12.2's
/// sync benchmarks write the target files themselves.
fn write_minimal_git(repo: &std::path::Path) -> io::Result<()> {
    let git = repo.join(".git");
    fs::create_dir_all(git.join("refs").join("heads"))?;
    fs::create_dir_all(git.join("objects"))?;
    fs::write(git.join("HEAD"), b"ref: refs/heads/main\n")?;
    fs::write(
        git.join("config"),
        b"[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The module is exercised end-to-end by the integration suite
    /// (`tests/temp_repos.rs` — layout, canonical paths, keys and the
    /// ALG-052 scan check); the unit here pins only the pure naming
    /// of the roots, which the fs-less store side also relies on.
    #[test]
    fn root_names_are_index_ordered_and_unique() {
        let repos = TempRepos::create(3, "unit-naming").expect("the fixture must create");
        let names: Vec<String> = repos
            .roots()
            .iter()
            .map(|root| {
                root.path
                    .file_name()
                    .expect("a canonical root has a file name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["repo-0000", "repo-0001", "repo-0002"]);
    }

    /// A label that is not one safe component is refused up front —
    /// a separator would nest the group, `..` would escape the
    /// guard's base and survive the cleanup.
    #[test]
    fn unsafe_labels_are_refused() {
        for label in ["", "a/b", "a\\b", "..", "../escape", "a/../b"] {
            let error = TempRepos::create(1, label).expect_err("an unsafe label must be rejected");
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::InvalidInput,
                "label {label:?} must be an InvalidInput refusal"
            );
        }
    }
}
