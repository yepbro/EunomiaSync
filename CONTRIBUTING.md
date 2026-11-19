# Contributing to EunomiaSync

First off, thank you for considering contributing to EunomiaSync! It's people like you that make this project better for everyone.

## Code of Conduct

By participating in this project, you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md). Please read it before interacting with the project.

## How Can I Contribute?

### Reporting Bugs

Bugs are tracked as [GitHub issues](https://github.com/yepbro/EunomiaSync/issues). Before creating a bug report, please check that the issue has not already been reported. When creating a bug report, please include:

- A clear and descriptive title
- Steps to reproduce the problem
- Expected and actual behavior
- Your operating system, application version, and other relevant environment details
- Screenshots or logs, if applicable

### Suggesting Enhancements

Enhancement suggestions are also tracked as GitHub issues. Please provide:

- A clear and descriptive title
- A detailed description of the proposed feature
- Explain why this enhancement would be useful to most users

For significant changes, please open an issue for discussion before starting implementation. This helps avoid investing time in changes that may not align with the project's direction.

### Pull Requests

1. Fork the repository and create your branch from `main`.
2. If you've added code that should be tested, add tests.
3. Ensure the test suite passes and the code is properly formatted (see below).
4. Keep pull requests focused on a single change whenever possible.
5. Provide a clear description of the problem and the proposed solution.
6. Link related issues, if applicable.
7. Be prepared to address review feedback.

All contributions must comply with the project's [Contributor License Agreement](CLA.md). See the **Contributor License Agreement (CLA)** section below.

## Contributor License Agreement (CLA)

Before a contribution can be merged, its author must explicitly accept the project's [Contributor License Agreement](CLA.md).

The CLA is intended to:

- Allow the project to distribute contributions under its existing open-source licenses.
- Allow the project owner to offer the project, including contributions, under additional licenses, including commercial or proprietary licenses.
- Provide the necessary copyright and patent permissions while allowing contributors to retain ownership of their contributions.

**What this means for contributors:**

- You retain the copyright to your contributions.
- You grant the project owner the rights described in the CLA.
- You do not give up your right to use your own contributions elsewhere.
- You are not required to provide ongoing support or maintenance for your contributions.
- You must have the legal right to submit the contribution and agree to the CLA.

Please read [CLA.md](CLA.md) before submitting a pull request.

The CLA must be accepted explicitly through the project's designated acceptance process. Merely submitting a pull request does not constitute acceptance.

If you have previously contributed to the project, you may be asked to accept the CLA for your earlier contributions as well. Contributions not covered by an accepted CLA may remain subject to their original licensing terms.

## Development Setup

The project is a cross-platform desktop application built with Rust, Tauri 2, and Vue 3.

Prerequisites:

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) (LTS) with [pnpm](https://pnpm.io/) (install via `corepack enable` or the [standalone script](https://pnpm.io/installation))
- Tauri 2 system dependencies (see the [Tauri prerequisites](https://tauri.app/start/prerequisites/))

```bash
git clone git@github.com:yepbro/EunomiaSync.git
cd EunomiaSync
pnpm install
pnpm run tauri dev    # run the app in development mode
```

## Code Style and Checks

Before submitting a pull request, please make sure all checks pass:

```bash
cargo fmt --all -- --check   # check Rust formatting
cargo clippy --workspace --all-targets -- -D warnings   # lint Rust code
cargo deny check             # lint dependencies (licenses, advisories, duplicates)
cargo test --workspace       # run Rust tests
pnpm run lint                # lint frontend code (ESLint)
pnpm run format:check        # check frontend formatting (Prettier)
pnpm run typecheck           # TypeScript type checking
pnpm run test                # run frontend unit tests (Vitest)
pnpm run e2e                 # run browser end-to-end tests (Playwright)
```

### Running Rust tests on Windows

`cargo test --workspace` on Windows currently dies at process start with
`STATUS_ENTRYPOINT_NOT_FOUND` (`0xc0000139`): the test executables link
without an application manifest — tauri-build embeds it into `[[bin]]`
targets only ([tauri-apps/tauri#13419](https://github.com/tauri-apps/tauri/issues/13419))
— so the loader binds Common-Controls v5 and the v6-only
`TaskDialogIndirect` import (via tao/tauri) cannot resolve. CI stages the
app manifest beside the test executables (see the staging step in
`.github/workflows/ci.yml` and DECISIONS.md D-074); locally, copy it the
same way before running the tests:

```powershell
Get-ChildItem target\debug\build\eunomia-sync -Recurse -Filter *.exe -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -match '\\out\\' } |
  ForEach-Object { Copy-Item src-tauri\windows\app.manifest "$($_.FullName).manifest" -Force }
Get-ChildItem target\debug\deps -Filter 'eunomia_sync*.exe' -ErrorAction SilentlyContinue |
  ForEach-Object { Copy-Item src-tauri\windows\app.manifest "$($_.FullName).manifest" -Force }
```

The first glob covers cargo 1.100+ (its build-dir layout v2 places test
executables in `build\<pkg>\<hash>\out`), the second the older `deps\`
layout. Delete this section when a durable fix lands upstream.

### Pre-commit Hooks

Git pre-commit hooks are installed automatically: running `pnpm install` executes the `prepare` script, which sets `git config core.hooksPath .githooks`.

The hook runs `cargo fmt --check`, `pnpm run lint`, `pnpm run format:check`, and `pnpm run typecheck` before each commit, so most style issues are caught locally before they reach CI.

## Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```text
<type>(<scope>): <short summary>

<optional body>
```

Common types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`.

## Review Process

- Maintainers will review your pull request and may request changes or provide feedback.
- Small, focused pull requests are reviewed and merged faster than large ones.
- Maintainers may ask for additional tests, documentation, or clarification.
- A pull request can only be merged after it has passed the required checks, received maintainer approval, and the contributor has completed the CLA acceptance process.
- Once approved, a maintainer will merge the changes.

## License

EunomiaSync is distributed under the MIT and Apache-2.0 open-source licenses.

By contributing, you agree that your contributions may be distributed under these licenses and that the project owner may also distribute the project under additional license terms, as described in the [Contributor License Agreement](CLA.md).

The CLA does not revoke or restrict rights already granted to recipients under the MIT or Apache-2.0 licenses.
