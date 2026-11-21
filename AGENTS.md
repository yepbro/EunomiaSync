# AGENTS.md

Instructions for coding agents working in this repository.

Main-window prototype (html-pages branch): https://es-demo-634.netlify.app

## 1. Read Local Instructions First

If `AGENTS.local.md` exists in the repository root, read it before doing
anything else and follow its instructions.

## 2. Worktree Bootstrap

After creating a worktree (`git worktree add <path> <branch>`), run
`pnpm install` in the worktree root. pnpm hardlinks packages from its global
content-addressable store, so a fresh worktree is ready in seconds and costs
only a few MiB of new disk (measured ~0.5 s / ~8 MiB — see DECISIONS.md
D-018). Hardlinks require the worktree to be on the same filesystem as the
store (`~/.local/share/pnpm`); on another filesystem pnpm falls back to
copying (npm-like cost). The install also runs the `prepare` script, which
sets `git config core.hooksPath .githooks` — once it finishes, the pre-commit
hook (`.githooks/pre-commit`) works with no further setup.

## 3. Project Rules

Distilled from DECISIONS.md; each rule exists because its violation has
already cost the project a fix. Every rule is checkable at review.

1. **Resolve spec ambiguity before coding.** A requirement open to more
   than one reading is written down as a question first; the chosen
   reading lands in DECISIONS.md before implementation.
2. **Verify facts, never assume them.** A claim about a crate's API or a
   platform's behavior counts only after checking the installed version's
   sources or a compile check; cite the source (file/line) in the record.
3. **Platform facts live behind gates.** OS error codes, path prefixes,
   manifests: cfg gates in production code; test expectations are built
   with the project's own helpers, never their std twins.
4. **Every test must be falsifiable.** An oracle is independent (a spec
   literal, an external vector), never a mirror of the implementation.
   Verify a new test by mutation: break the code — the test must fail.
5. **Green means exit code.** Check chains gate on the real exit status
   (`$?` or `PIPESTATUS`); `cmd | tail/grep && …` shapes are forbidden.
6. **Untrusted input gets stated complexity.** Processing text and paths
   from files or the database is linear; edge cases are covered by a
   test on a large input.
7. **A guard protects its whole region.** A lock/guard lives across the
   entire region it protects, `.await` included; take-and-immediately-
   drop is a bug.
8. **User content never reaches diagnostics.** Logs and errors carry
   paths, codes and numbers; arbitrary stored text is never interpolated
   (class-only diagnostics).
9. **Dependencies and unsafe are owner decisions.** A new crate, feature
   or unsafe block requires an explicit owner ruling; cargo-deny stays
   green with no new ignores.
10. **Frozen zones stay untouched.** A file owned by a parallel task is
    not edited; a forced deviation (e.g. regenerated bindings) is
    recorded in DECISIONS.md.
11. **Docs must not lie.** Changing behavior updates the claims about it
    in the same change; what automation cannot prove goes to the manual
    smoke checklist, not a pseudo-test.
