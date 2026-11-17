# EunomiaSync — Agent Instructions Manager

EunomiaSync helps developers maintain accurate and up-to-date `AGENTS.md` files across multiple projects. It brings shared instructions and project-specific variations into one manageable system, accounting for differences in tech stacks, development methodologies, conventions, and workflows. It also makes it easy to introduce new rules across all relevant projects without manually updating each file.

It is a cross-platform desktop client built in Rust, running natively on Windows, macOS, and Linux.

<p align="center">
  <img src="logo.png" alt="EunomiaSync — Agent Instructions Manager" width="100%">
</p>

### Why the name EunomiaSync?

**Eunomia** comes from Greek mythology and represents good order, sound governance, and the proper arrangement of things.

This reflects the core purpose of the application: bringing order to a growing collection of project instructions, keeping shared rules consistent while respecting the unique needs of each project.

**Sync** represents keeping `AGENTS.md` files across projects aligned with their source instructions, making it easy to roll out new rules and keep everything up to date as projects evolve.

Together, **EunomiaSync** represents order, consistency, and effortless rule management across multiple projects — without forcing them all to follow exactly the same rules.

### Tech Stack

| Component          | Technology           |
|--------------------|----------------------|
| Backend            | Rust                 |
| Desktop framework  | Tauri 2              |
| Frontend           | Vue 3 + TypeScript   |
| UI                 | Tailwind CSS         |
| Database           | SQLite               |
| Markdown           | pulldown-cmark       |
| Diff               | similar              |
| File watching      | notify               |
| Config             | serde + YAML/TOML    |

## License

EunomiaSync is dual-licensed under your choice of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
