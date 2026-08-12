# Contributing

Beacon is early-stage software. Protocol and security changes require tests and
documentation updates.

Before opening a pull request:

- explain the behavior change;
- add or update tests;
- do not include secrets or private infrastructure details;
- document compatibility impact;
- run formatting, lint, and tests locally when the Rust toolchain is available.

The CI gate runs the following commands:

```sh
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```
