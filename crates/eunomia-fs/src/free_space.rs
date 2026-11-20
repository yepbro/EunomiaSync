//! Free space on the data-directory volume.
//!
//! Spec 9.4 (phase 1, the closing rule): "Free space for backups is
//! checked on the volume of the data directory: if there is less space
//! than the total size of the copies plus a 20% margin, the run ends as
//! `aborted` with `E_DISK_FULL`." The backups live in the data directory
//! (spec 7.6, BKP-001), so the volume that must be checked is the data
//! directory's — not the projects' volume (the §10 `E_DISK_FULL` row
//! names the project-volume variant too, but that half belongs to the
//! write phase, a later task). The `E_DISK_FULL` wire mapping itself
//! belongs to the command layer and stays out of this module, exactly
//! like every other code mapping (the D-062 posture).
//!
//! The module is three pieces:
//!
//! - [`required_for`] — the pure arithmetic: the total size of the
//!   future copies plus a 20% margin ([`MARGIN_PERCENT`], a spec-fixed
//!   constant, no settings read).
//! - [`check_space`] — the pure decision: is there at least that much
//!   free space. No I/O, no clock, no configuration.
//! - [`check_backup_space`] — the seam the sync pipeline will call: the
//!   caller supplies the free-space source as a closure, so tests (and
//!   the card's acceptance test) substitute a fake without touching a
//!   filesystem, while production passes `free_bytes` over the data
//!   directory.
//!
//! # Probe failures are typed, semantics stay with the caller
//!
//! §9.4's rule is about the RUN outcome, which is the pipeline's concern:
//! this module only reports what it observed. A failed volume query is
//! not "no space" — it surfaces as [`FreeSpaceError::ProbeFailed`] with
//! the OS code riding in the source error (§10's first-class log fact),
//! and [`FreeSpaceError::ProbeUnavailable`] marks the platform where no
//! production probe exists yet (see "Windows" below). The caller decides
//! skip/abort semantics per variant.
//!
//! # Windows
//!
//! The production probe is `#[cfg(unix)]`. **rustix 1.1.6 has no `fs`
//! module on Windows at all** (`#[cfg(not(windows))] pub mod fs;` in its
//! lib.rs), so the pre-ruled probe simply cannot exist there, and the
//! brief forbids substituting a dependency on my own authority. The
//! `#[cfg(windows)]` arm of [`free_bytes`] exists only so the crate
//! compiles on the Windows CI leg; it returns
//! [`FreeSpaceError::ProbeUnavailable`] — no fabricated data, no unsafe,
//! no new dependency. This is an ESCALATION POINT recorded in
//! DECISIONS.md for the orchestrator, not a settled design: the Windows
//! production call (e.g. `GetDiskFreeSpaceExW` via the already-locked
//! `windows-sys`, an unsafe-FFI decision) is the owner's to sanction.
//!
//! # Testing plan
//!
//! The card's acceptance test is the substituted-source seam: fakes
//! drive the whole decision path (enough, exactly-short, one-byte-short,
//! way-short, probe-error passthrough) with no filesystem involved. The
//! margin arithmetic is pinned exactly (20% of the sum, floor at the
//! byte, saturation at `u64::MAX` where a wrapping bug would turn a
//! full disk into "fits"). The `bavail` × `frsize` ruling is pinned
//! against hand-built statvfs fixtures, not a live volume; the real
//! probe gets one smoke test on a temp directory.

use std::fmt;
use std::io;
use std::path::Path;

/// The spec-fixed margin over the total size of the copies, in percent
/// (spec 9.4: "the total size of the copies plus a 20% margin"). A named
/// constant, not a setting: §9.4 fixes the number, and the module reads
/// no configuration.
pub const MARGIN_PERCENT: u64 = 20;

/// The space the sync pipeline must verify on the data-directory volume:
/// the sum of the future backup copies plus a 20% margin of that sum,
/// added on top.
///
/// Spec 9.4 phase 1: the check compares "the total size of the copies
/// plus a 20% margin" against the free space. The margin is 20% OF the
/// sum (not of the volume, not of the free space), rounded down to whole
/// bytes at the division, and the final total saturates at `u64::MAX`
/// instead of wrapping — a near-`u64::MAX` copy list must demand the
/// ceiling, never wrap into a tiny "fits" number.
///
/// An empty copy list requires zero bytes; whether the pipeline consults
/// the probe at all for a backupless run is §9.4 phase logic, not this
/// function's.
pub fn required_for(copies: &[u64]) -> u64 {
    let total = total_of(copies);
    // `sum × 20 / 100` in `u128`: the multiplication is exact for every
    // `u64` total (an `saturating_mul`-then-divide would distort the
    // margin near the ceiling), and the quotient is one fifth of the
    // sum, so it always fits `u64`.
    let margin = u128::from(total) * u128::from(MARGIN_PERCENT) / 100;
    // `sum + margin` exceeds `u64` only above five sixths of the
    // ceiling; saturate there — the requirement cannot be met on any
    // representable volume, and saturation keeps the number monotone.
    u64::try_from(u128::from(total) + margin).unwrap_or(u64::MAX)
}

/// The sum of the copy sizes, saturating: a list whose total overflows
/// `u64` demands the ceiling, never wraps into a small number.
fn total_of(copies: &[u64]) -> u64 {
    copies
        .iter()
        .fold(0u64, |acc, size| acc.saturating_add(*size))
}

/// The pure decision of §9.4: does `free` cover `required`.
///
/// The spec aborts only "if there is LESS space than the total size of
/// the copies plus a 20% margin" — so exact equality passes, and the
/// shortage is strictly `free < required`. No I/O: the numbers are
/// given, whoever computed them. Both numbers ride in the error for the
/// §10 message ("Free up space" — the caller can show the shortfall).
///
/// # Errors
///
/// [`FreeSpaceError::NotEnoughSpace`] when `free < required`; the `free`
/// and `required` fields carry exactly the arguments. `Ok(())` when the
/// space suffices, including the exact-equality boundary and the
/// `0 == 0` empty case.
pub fn check_space(free: u64, required: u64) -> Result<(), FreeSpaceError> {
    if free < required {
        Err(FreeSpaceError::NotEnoughSpace { free, required })
    } else {
        Ok(())
    }
}

/// Checks that the data-directory volume can hold the backups of this
/// run, with the free-space source injected.
///
/// This is the §9.4 phase-1 seam: the pipeline computes the copy sizes,
/// supplies a source for the free space — in production
/// `|| free_bytes(&data_dir)` — and this function runs the whole
/// decision path: sum, margin, comparison. The source is a plain
/// `FnOnce`, the weakest contract that works, so a test substitutes a
/// fake with no filesystem at all (the card's acceptance test).
///
/// # Errors
///
/// - [`FreeSpaceError::ProbeFailed`] / [`FreeSpaceError::ProbeUnavailable`]
///   pass through verbatim when the SOURCE fails — including for an empty
///   copy list: the source is consulted first, a failed query fails the
///   check regardless of how little space would be needed, and the caller
///   decides what a run does about it (§9.4's `aborted` outcome is the
///   pipeline's wording, not this function's).
/// - [`FreeSpaceError::NotEnoughSpace`] when the source reported enough
///   information and the decision came up short.
pub fn check_backup_space<S>(copies: &[u64], free_space: S) -> Result<(), FreeSpaceError>
where
    S: FnOnce() -> Result<u64, FreeSpaceError>,
{
    let free = free_space()?;
    check_space(free, required_for(copies))
}

/// Queries the free space on the volume holding `path`, in bytes.
///
/// The production probe behind [`check_backup_space`]'s seam. The bytes
/// that matter are what THIS process could still allocate for the
/// unprivileged write of a backup copy: `f_bavail` ("available to
/// unprivileged users") times `f_frsize` (the fundamental block size the
/// block counts are denominated in) — not `f_bfree` (total free,
/// including the reserved blocks that keep the filesystem healthy and
/// that a non-root write must not consume) and not `f_bsize` (the
/// "optimal transfer size", which need not equal the block-count unit).
/// The multiplication saturates: a statvfs that reports absurd values
/// yields the `u64` ceiling, not a wrapped number.
///
/// The data directory is the caller's argument (spec 5.5): this module
/// knows nothing about how the path is produced, so no settings and no
/// database are read here.
///
/// # Platform support
///
/// Unix (Linux and macOS, the pre-ruled `rustix::fs::statvfs`). On
/// Windows the function returns [`FreeSpaceError::ProbeUnavailable`] —
/// rustix has no `fs` module there; see the module docs and the
/// DECISIONS.md entry for the escalation.
///
/// # Errors
///
/// [`FreeSpaceError::ProbeFailed`] when the volume query itself fails:
/// the path is missing, access is denied, the drive dropped — the
/// [`io::Error`] rides in the variant with its OS code. Never panics.
#[cfg(unix)]
pub fn free_bytes(path: &Path) -> Result<u64, FreeSpaceError> {
    // rustix's linux_raw backend answers with `io::Errno`; the std
    // conversion keeps the OS code as `io::Error::raw_os_error` (the
    // shape §10's log line and `ErrorDetails::from_io_error` consume),
    // and `From<io::Error>` wraps it in the typed error.
    let stats =
        rustix::fs::statvfs(path).map_err(|errno| FreeSpaceError::from(io::Error::from(errno)))?;
    Ok(available_bytes(&stats))
}

/// The Windows arm exists only so the crate compiles on the Windows CI
/// leg (D-060's four-OS matrix): rustix 1.1.6 has no `fs` module on
/// Windows, and no substitute is sanctioned yet. It fabricates no data —
/// the caller sees [`FreeSpaceError::ProbeUnavailable`] and decides the
/// run semantics, exactly as for any other probe failure. ESCALATION
/// POINT: the production call (e.g. `GetDiskFreeSpaceExW` over the
/// already-locked `windows-sys`) is recorded in DECISIONS.md for the
/// orchestrator.
///
/// # Errors
///
/// Always [`FreeSpaceError::ProbeUnavailable`].
#[cfg(windows)]
pub fn free_bytes(_path: &Path) -> Result<u64, FreeSpaceError> {
    Err(FreeSpaceError::ProbeUnavailable)
}

/// The `bavail` × `frsize` arithmetic, split from the syscall so a test
/// can pin it against a hand-built fixture instead of a live volume.
#[cfg(unix)]
fn available_bytes(stats: &rustix::fs::StatVfs) -> u64 {
    stats.f_bavail.saturating_mul(stats.f_frsize)
}

/// Why the free-space check said no.
///
/// One variant per situation the §9.4 phase-1 check can end in, so the
/// command layer can map [`FreeSpaceError::NotEnoughSpace`] to
/// `E_DISK_FULL` (§10: "No space: on the backups volume in phase 1") and
/// log the other two with their OS facts without re-deriving them from
/// strings. The probe-backed variant carries the [`io::Error`] itself as
/// its `source` — its `raw_os_error` and `ErrorKind` are §10's
/// first-class log facts, and `ErrorDetails::from_io_error` in the app
/// layer consumes exactly that shape.
#[derive(Debug)]
pub enum FreeSpaceError {
    /// The volume cannot hold the copies plus the margin: `free` is
    /// strictly less than `required` (spec 9.4). The command layer maps
    /// this to `E_DISK_FULL` and can show the shortfall.
    NotEnoughSpace {
        /// The free space the source reported, in bytes.
        free: u64,
        /// The demanded total (sum + 20% margin), in bytes.
        required: u64,
    },
    /// The volume query failed: the path is missing, access is denied,
    /// the drive dropped — every OS refusal of the probe. This is NOT a
    /// "no space" verdict; the caller decides the run semantics.
    ProbeFailed {
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The platform has no production probe (Windows, pending the
    /// orchestrator ruling recorded in DECISIONS.md). No OS call was
    /// made, so no OS code exists for this verdict.
    ProbeUnavailable,
}

impl FreeSpaceError {
    /// The I/O error behind this failure, when the OS refused the probe;
    /// `None` for the verdicts that are plain facts (the shortage
    /// numbers, the platform gap) rather than OS failures.
    pub fn io_error(&self) -> Option<&io::Error> {
        match self {
            FreeSpaceError::ProbeFailed { source } => Some(source),
            FreeSpaceError::NotEnoughSpace { .. } | FreeSpaceError::ProbeUnavailable => None,
        }
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`): §10
    /// names it a first-class fact of every error log line. `None`
    /// where no OS call failed.
    pub fn os_code(&self) -> Option<i32> {
        self.io_error().and_then(io::Error::raw_os_error)
    }
}

impl From<io::Error> for FreeSpaceError {
    fn from(source: io::Error) -> Self {
        FreeSpaceError::ProbeFailed { source }
    }
}

impl fmt::Display for FreeSpaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FreeSpaceError::NotEnoughSpace { free, required } => {
                write!(
                    f,
                    "not enough free space: {free} bytes available, {required} required"
                )
            }
            FreeSpaceError::ProbeFailed { source } => {
                write!(f, "free-space probe failed: {source}")
            }
            FreeSpaceError::ProbeUnavailable => {
                write!(f, "free-space probe is unavailable on this platform")
            }
        }
    }
}

impl std::error::Error for FreeSpaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io_error()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    /// A fake free-space source for the substituted-source acceptance
    /// tests: a closure over a plain number, no filesystem anywhere.
    fn fake_free(free: u64) -> impl FnOnce() -> Result<u64, FreeSpaceError> {
        move || Ok(free)
    }

    #[test]
    fn enough_space_passes_through_the_substituted_source() {
        // Two copies of 100 and 200 bytes: the total is 300, the margin
        // is 60, the demand is 360 — 400 free covers it. The card's
        // acceptance path: the fake drives sum, margin and decision in
        // one call, with no filesystem involved.
        check_backup_space(&[100, 200], fake_free(400)).expect("400 free covers 300 + 20% = 360");
    }

    #[test]
    fn exactly_the_required_space_passes() {
        // Spec 9.4 aborts only when there is LESS space than the total
        // plus margin; the wording makes exact equality a pass.
        check_backup_space(&[100, 200], fake_free(360))
            .expect("the boundary is free == required, and it passes");
        check_space(0, 0).expect("the degenerate 0 == 0 case passes too");
    }

    #[test]
    fn one_byte_short_fails_with_the_exact_numbers() {
        let err = check_backup_space(&[100, 200], fake_free(359))
            .expect_err("359 is strictly less than the required 360");
        assert!(
            matches!(
                &err,
                FreeSpaceError::NotEnoughSpace {
                    free: 359,
                    required: 360
                }
            ),
            "the shortage must carry the exact numbers, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            None,
            "the shortage is a decision, no OS call made"
        );
    }

    #[test]
    fn way_short_fails_with_the_full_numbers_carried() {
        // Megabyte-scale copies against a nearly empty volume: the
        // verdict, the Display text and the numbers all pin the path a
        // real full disk would take.
        let copies = [1_000_000, 2_000_000];
        let err =
            check_backup_space(&copies, fake_free(10)).expect_err("10 bytes cannot cover 3.6 MB");
        assert!(
            matches!(
                &err,
                FreeSpaceError::NotEnoughSpace {
                    free: 10,
                    required: 3_600_000
                }
            ),
            "the verdict must carry free and required, got: {err:?}"
        );
        assert!(
            err.to_string().starts_with("not enough free space: "),
            "the Display text must state the fact: {err}"
        );
    }

    #[test]
    fn the_margin_is_twenty_percent_of_the_sum_added_on_top() {
        // 3_000_000 of copies: the margin is 600_000 (20% OF the sum),
        // the demand is 3_600_000 — not 20% of anything else.
        assert_eq!(required_for(&[1_000_000, 2_000_000]), 3_600_000);
        assert_eq!(required_for(&[3_000_000]), 3_600_000);
    }

    #[test]
    fn the_margin_floors_the_percent_of_an_odd_total() {
        // 101 bytes: 20% is 20.2, floored to 20 at the division — the
        // demand is 121, pinning the arithmetic byte for byte.
        assert_eq!(required_for(&[101]), 121);
        assert_eq!(required_for(&[99]), 118, "99 + floor(19.8) = 118");
    }

    #[test]
    fn no_copies_require_no_space() {
        assert_eq!(required_for(&[]), 0);
        check_backup_space(&[], fake_free(0))
            .expect("a run with nothing to back up fits on a full volume");
    }

    #[test]
    fn near_max_totals_saturate_instead_of_wrapping_into_fits() {
        // The wrap bug this pins: [u64::MAX - 10, 20] summed with `+`
        // would wrap to 9, the margin would vanish, and the demand would
        // be 9 bytes — a full disk would read as "fits". Saturation
        // keeps the demand at the ceiling instead.
        let required = required_for(&[u64::MAX - 10, 20]);
        assert_eq!(required, u64::MAX, "the demand saturates at the ceiling");
        check_space(u64::MAX - 1, required)
            .expect_err("anything below the ceiling must still fail the check");
        check_space(u64::MAX, u64::MAX).expect("the ceiling itself is the exact-equality boundary");

        // A single copy at the ceiling: the margin alone would overflow
        // the sum, the total saturates, nothing wraps.
        assert_eq!(required_for(&[u64::MAX]), u64::MAX);
    }

    #[test]
    fn the_margin_multiplication_is_exact_below_the_ceiling() {
        // A single five-exabyte copy: 20% of it is exactly one exabyte,
        // still representable, so the demand is 6 EB. A
        // saturating_mul-then-divide shortcut would clamp the margin at
        // u64::MAX/100 (about 184 PB) and demand 5_184_467_440_737_095_516
        // instead — this pin is what keeps the arithmetic exact in u128
        // rather than "close enough near the ceiling".
        assert_eq!(
            required_for(&[5_000_000_000_000_000_000]),
            6_000_000_000_000_000_000
        );
    }

    #[test]
    fn a_probe_error_passes_through_untouched() {
        // ENOSPC from the probe itself is not a "no space" verdict: the
        // source's error must surface verbatim, the decision must never
        // run on data that was never fetched.
        let source = || -> Result<u64, FreeSpaceError> {
            Err(FreeSpaceError::ProbeFailed {
                source: io::Error::from_raw_os_error(28),
            })
        };
        let err =
            check_backup_space(&[100], source).expect_err("a failing source must fail the check");
        assert!(
            matches!(&err, FreeSpaceError::ProbeFailed { .. }),
            "the probe error must pass through, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(28),
            "ENOSPC = 28 rides in the typed error for the §10 log line"
        );
    }

    #[test]
    fn a_probe_unavailable_verdict_passes_through_untouched() {
        // The Windows arm's verdict (pending the orchestrator ruling
        // recorded in D-089) is a probe failure like any other: the seam
        // surfaces it verbatim and the caller decides the run semantics
        // — the contract must hold on the platform where the variant is
        // produced.
        let source = || -> Result<u64, FreeSpaceError> { Err(FreeSpaceError::ProbeUnavailable) };
        let err = check_backup_space(&[100], source).expect_err("an unavailable probe must fail");
        assert!(
            matches!(&err, FreeSpaceError::ProbeUnavailable),
            "the verdict must pass through, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            None,
            "no OS call was made, so no OS code exists"
        );
    }

    #[test]
    fn the_source_is_consulted_even_for_an_empty_copy_list() {
        // Probe-first order, pinned: an empty list requires zero, but a
        // failed query still fails the check — reordering to skip the
        // probe when nothing is needed would flip this verdict.
        let source = || -> Result<u64, FreeSpaceError> {
            Err(FreeSpaceError::ProbeFailed {
                source: io::Error::from_raw_os_error(2),
            })
        };
        let err = check_backup_space(&[], source)
            .expect_err("a failed query fails the check even with nothing to back up");
        assert!(
            matches!(&err, FreeSpaceError::ProbeFailed { .. }),
            "the probe error must surface verbatim, got: {err:?}"
        );
    }

    #[test]
    fn an_io_error_maps_into_the_probe_variant() {
        // The From impl the production probe uses: an io::Error becomes
        // the ProbeFailed variant, io-backed, with its source chain.
        let err = FreeSpaceError::from(io::Error::from_raw_os_error(13));
        assert!(
            matches!(&err, FreeSpaceError::ProbeFailed { .. }),
            "From<io::Error> must produce ProbeFailed, got: {err:?}"
        );
        assert_eq!(err.os_code(), Some(13), "EACCES = 13 rides along");
        assert!(
            err.to_string().starts_with("free-space probe failed: "),
            "the Display text must state the fact, then the OS text: {err}"
        );
    }

    #[cfg(unix)]
    fn statvfs_fixture(bavail: u64, bfree: u64, frsize: u64, bsize: u64) -> rustix::fs::StatVfs {
        rustix::fs::StatVfs {
            f_bsize: bsize,
            f_frsize: frsize,
            f_blocks: bfree + 500,
            f_bfree: bfree,
            f_bavail: bavail,
            f_files: 1000,
            f_ffree: 900,
            f_favail: 900,
            f_fsid: 0,
            f_flag: rustix::fs::StatVfsMountFlags::empty(),
            f_namemax: 255,
        }
    }

    #[cfg(unix)]
    #[test]
    fn bavail_times_frsize_pinned_against_a_hand_built_fixture() {
        // 100 available blocks × 4096-byte fundamental blocks = 409_600
        // bytes. The fixture carries 1000 TOTAL-free blocks: if the
        // arithmetic used bfree instead of bavail, the result would be
        // 4_096_000 — the reserved-block difference the ruling exists
        // for.
        let fixture = statvfs_fixture(100, 1000, 4096, 4096);
        assert_eq!(
            available_bytes(&fixture),
            409_600,
            "the probe must count bavail × frsize, not bfree"
        );

        // The multiplication saturates: an absurd fixture yields the
        // u64 ceiling, never a wrapped value and never a panic.
        let absurd = statvfs_fixture(u64::MAX / 2, u64::MAX / 2, 5, 5);
        assert_eq!(available_bytes(&absurd), u64::MAX);
    }

    #[cfg(unix)]
    #[test]
    fn frsize_not_bsize_is_the_block_unit() {
        // statvfs allows bsize ("optimal transfer size") to differ from
        // frsize (the fundamental block size the counts use): 100
        // available blocks of 4096 bytes are 409_600 bytes, not
        // 819_200, whatever the transfer size says.
        let fixture = statvfs_fixture(100, 1000, 4096, 8192);
        assert_eq!(
            available_bytes(&fixture),
            409_600,
            "block counts are denominated in frsize, not bsize"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_real_probe_reports_a_positive_amount_on_a_live_directory() {
        // Sanity pin that the rustix path works end to end; the
        // substituted-source tests above are the contract, this is the
        // wiring check.
        let dir = tempfile::TempDir::new().expect("a temporary directory must be creatable");
        let free = free_bytes(dir.path()).expect("statvfs on a live directory must succeed");
        assert!(free > 0, "a mountable volume reports a positive amount");
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_path_surfaces_the_os_error() {
        let dir = tempfile::TempDir::new().expect("a temporary directory must be creatable");
        let gone = dir.path().join("never-created");

        let err =
            free_bytes(&gone).expect_err("a probe on a missing path must fail with the OS error");
        assert!(
            matches!(&err, FreeSpaceError::ProbeFailed { .. }),
            "a missing path must be ProbeFailed, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(2),
            "ENOENT = 2 is the OS code the §10 log line carries"
        );
    }

    #[test]
    fn display_texts_and_the_source_chain_follow_the_contract() {
        // The command layer branches on the variants and logs the
        // Display text; these assertions pin the wording (lowercase, no
        // trailing punctuation — the house style) and the source()
        // chain without pinning std's strerror table.
        let shortage = FreeSpaceError::NotEnoughSpace {
            free: 10,
            required: 360,
        };
        assert_eq!(
            shortage.to_string(),
            "not enough free space: 10 bytes available, 360 required"
        );
        let unavailable = FreeSpaceError::ProbeUnavailable;
        assert_eq!(
            unavailable.to_string(),
            "free-space probe is unavailable on this platform"
        );

        let probe = FreeSpaceError::ProbeFailed {
            source: io::Error::from_raw_os_error(13),
        };
        assert!(
            probe.to_string().starts_with("free-space probe failed: "),
            "the ProbeFailed text must state the fact, then the OS text: {probe}"
        );

        // source() chains exactly to the carried io error on the
        // probe-backed variant, and is None where no OS call failed.
        assert_eq!(
            probe.source().map(|source| source.to_string()),
            Some(io::Error::from_raw_os_error(13).to_string())
        );
        assert!(shortage.source().is_none());
        assert!(unavailable.source().is_none());
    }
}
