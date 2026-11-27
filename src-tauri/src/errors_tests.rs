//! Acceptance pins for the `AppError` export (spec §7.10, §10; T-013).
//!
//! `AppError` reaches `src/bindings.ts` through the tauri-specta
//! builder's `typ` registration in `lib.rs` — not through a command
//! signature (no registered command exists yet; see the `errors.rs`
//! module docs).
//! These pins hold that export to the task's acceptance criteria:
//!
//! - the committed `src/bindings.ts` (embedded at compile time, like
//!   every file pin in `config_tests.rs`) exports the `AppError` type
//!   with its four §7.10 fields and **all twenty-one** registry code
//!   literals — and no other `E_*` literal, so the one remaining P1
//!   code of the §10 table (`E_IMPORT_INVALID`) stays out of the
//!   registry until its own task adds it;
//! - `lib.rs` keeps the `typ` registration wired (without it, the export
//!   test would silently *rewrite* the committed bindings to a version
//!   without the type — this pin is what turns that rewrite red).
//!
//! The literals below are spelled from the spec §10 table, never derived
//! from the enum — the same mutation-honesty as the `errors.rs` and
//! `logging.rs` suites.

/// The twenty-one registry error-code literals (spec §10: the T-013
/// card's twenty plus `E_RESTORE_SOURCE_MISSING`, whose row the P0
/// export `backup_save` turned on — BKP-012, T-353), in table order.
/// Test-side registry: the enum↔literal mapping is behavior-tested
/// in `errors.rs`; this list pins the *exported* form.
const MVP_CODE_LITERALS: [&str; 21] = [
    "E_SYNC_RUNNING",
    "E_NOT_A_REPO",
    "E_PROJECT_UNAVAILABLE",
    "E_NO_WRITE_ACCESS",
    "E_TARGET_IS_DIR",
    "E_TARGET_BROKEN_LINK",
    "E_TARGET_OUTSIDE_ROOT",
    "E_MARKERS_INVALID",
    "E_NOT_UTF8",
    "E_BACKUP_FAILED",
    "E_DISK_FULL",
    "E_FILE_LOCKED",
    "E_WRITE_FAILED",
    "E_VERIFY_FAILED",
    "E_CONTENT_RESERVED",
    "E_VALIDATION",
    "E_DB_LOCKED",
    "E_DB_CORRUPT",
    "E_DB_TOO_NEW",
    "E_RESTORE_SOURCE_MISSING",
    "E_UPDATE_FAILED",
];

/// The one §10 code that is still a P1 feature and therefore outside
/// the registry (module docs of `errors.rs`): library import. It
/// appears here as a literal precisely so its accidental addition
/// fails loudly instead of sliding in with an unrelated change.
/// (`E_RESTORE_SOURCE_MISSING` left this list with BKP-012/T-353 —
/// the export is its §10 row's P0 user.)
const P1_CODE_LITERALS: [&str; 1] = ["E_IMPORT_INVALID"];

/// Strips `/** ... */` doc comments from generated TypeScript. specta
/// exports the Rust doc comments, and they may contain `;`, `|` or even
/// quoted literals — prose that must not leak into the structural pins
/// below (the union block, for instance, is delimited by `;`, which the
/// `E_DB_CORRUPT` variant doc legitimately contains).
fn strip_ts_doc_comments(bindings: &str) -> String {
    let mut stripped = String::with_capacity(bindings.len());
    let mut rest = bindings;
    while let Some(start) = rest.find("/**") {
        stripped.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            // Unterminated doc comment: drop the rest (generated output
            // never has one; this keeps the helper total anyway).
            None => return stripped,
        }
    }
    stripped.push_str(rest);
    stripped
}

#[test]
fn bindings_export_the_app_error_type_with_all_twenty_one_codes() {
    // Arrange: the committed bindings file, embedded at compile time.
    let bindings = include_str!("../../src/bindings.ts");

    // Assert: the type itself is exported with its §7.10 field set
    // under the spec spellings (snake_case, exactly as §7.10 names
    // them), together with its nested types. specta-typescript emits
    // each struct as a `_Serialize | _Deserialize` pair (the two sides
    // differ on optional fields), so the pins use the emitted forms.
    for marker in [
        "export type AppError = ",
        "export type ErrorCode = ",
        "export type ErrorDetails = ",
        "code: ErrorCode",
        "message_key: string",
        "params: { [key in string]: string }",
        "details: ErrorDetails_Serialize | null",
        "details?: ErrorDetails_Deserialize | null",
    ] {
        assert!(
            bindings.contains(marker),
            "src/bindings.ts must contain `{marker}`: AppError is exported \
             through the specta builder's `typ` registration (spec §7.10)"
        );
    }

    // Assert: every registry code literal is present in the exported
    // `ErrorCode` union — the frontend sees the full registry — and the
    // union holds exactly those twenty-one literals, pinning the set in
    // both directions (a removed or added code fails the count). The
    // union block is read from the doc-stripped text and delimited by
    // the union's own `;`, so prose in exported doc comments cannot
    // shift the boundary or the count.
    let stripped = strip_ts_doc_comments(bindings);
    let union_start = stripped
        .find("export type ErrorCode = ")
        .expect("the marker check above guarantees the ErrorCode union");
    let union = stripped[union_start..]
        .split(';')
        .next()
        .expect("the ErrorCode union is terminated by `;`");
    for literal in MVP_CODE_LITERALS {
        assert!(
            union.contains(&format!("\"{literal}\"")),
            "src/bindings.ts must contain the error code `{literal}` in \
             the ErrorCode union (spec §10 registry)"
        );
    }
    let count = union.match_indices("\"E_").count();
    assert_eq!(
        count, 21,
        "the ErrorCode union must carry exactly the twenty-one registry \
         error-code literals, found {count}"
    );
}

#[test]
fn the_p1_codes_stay_out_of_the_registry() {
    // Arrange & Act: the committed bindings file, embedded at compile
    // time, with the exported doc comments stripped — the same honesty
    // as the union pin: a doc comment that merely *mentions* a P1 code
    // must not trip this check for the wrong reason.
    let bindings = strip_ts_doc_comments(include_str!("../../src/bindings.ts"));

    // Assert: the one remaining P1 code of the §10 table (library
    // import) is not in the registry — the enum and the export must
    // both stay without it until its own task lands. (The count half
    // of the presence pin above already makes it impossible; this pin
    // names it so the failure explains itself.)
    for literal in P1_CODE_LITERALS {
        assert!(
            !bindings.contains(literal),
            "src/bindings.ts must not contain `{literal}` yet: it is a P1 \
             code (spec §10) and joins the registry with its own task — \
             see the `errors.rs` module docs"
        );
    }
}

#[test]
fn lib_registers_app_error_with_the_specta_builder() {
    // Arrange & Act: `lib.rs`, compile-time constant, with comments
    // stripped so prose about the registration cannot satisfy the pin
    // (the shared `config_tests.rs` helper).
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));

    // Assert: the `typ` registration must appear exactly once, live.
    // Without it, `AppError` is unreachable from the export: the
    // `export_bindings_are_up_to_date` test would regenerate the
    // committed bindings *without* the type and stay green while doing
    // it — this pin is the tripwire that keeps the mechanism wired.
    assert_eq!(
        lib.matches(".typ::<errors::AppError>()").count(),
        1,
        "lib.rs must register AppError with the specta builder exactly \
         once via `.typ::<errors::AppError>()`: §7.10's error half of the \
         IPC contract reaches src/bindings.ts through it (D-003)"
    );
}
