# AGENTS.md

Instructions for coding agents working in this repository.

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
