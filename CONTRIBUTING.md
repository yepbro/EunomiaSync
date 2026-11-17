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

### Pull Requests

1. Fork the repository and create your branch from `main`.
2. If you've added code that should be tested, add tests.
3. Ensure the test suite passes and the code is properly formatted (see below).
4. Issue that pull request with a clear description of the problem and the solution.

## Development Setup

The project is a cross-platform desktop application built with Rust, Tauri 2, and Vue 3.

Prerequisites:

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) (LTS) with npm
- Tauri 2 system dependencies (see the [Tauri prerequisites](https://tauri.app/start/prerequisites/))

```bash
git clone git@github.com:yepbro/EunomiaSync.git
cd EunomiaSync
npm install
npm run tauri dev    # run the app in development mode
```

## Code Style and Checks

Before submitting a pull request, please make sure all checks pass:

```bash
cargo fmt --all          # format Rust code
cargo clippy --all-targets --all-features -- -D warnings   # lint Rust code
cargo test               # run Rust tests
npm run lint             # lint frontend code
npm run typecheck        # TypeScript type checking
```

## Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <short summary>

<optional body>
```

Common types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`.

## Review Process

- Maintainers will review your pull request and may request changes or provide feedback.
- Small, focused pull requests are reviewed and merged faster than large ones.
- Once approved, a maintainer will merge your changes.

## License

By contributing to EunomiaSync, you agree that your contributions will be licensed under the project's dual license (MIT OR Apache-2.0), with no additional terms.
