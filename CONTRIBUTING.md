# Contributing

Build from source using [docs/BUILDING.md](docs/BUILDING.md). Keep changes focused
and run `cargo test --locked --workspace`. Check Rust formatting with
`cargo fmt --all -- --check` and lint with
`cargo clippy --locked --workspace --all-targets -- -D warnings`.
Engine changes also require rebuilding the DLL and running the engine tests.
Installer changes require a fresh setup build.

Never include ARL values, browser cookies, saved sessions, downloaded music,
SDKs, generated reports or build outputs in a commit. Put release binaries in
GitHub Releases. Engine changes must retain its license and document their
relationship to upstream.
