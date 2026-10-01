# TinySearch repository guidance

This Rust 2024 workspace has two crates. `crates/tinysearch-bus` owns the
transport-free wire contract, names, payloads, and pure catalog selection.
`crates/tinysearch` owns provider behavior, routing, the TinyBus adapter, and
the installable cdylib. Keep provider HTTP behavior out of the bus crate.

Do work on a feature branch in a worktree. Keep `vendor/tinybus` pinned as a
submodule; change its source in its own repository. Do not commit credentials.
Do not log provider queries, credentials, or result content. Provider credentials
arrive through sensitive module initialization and reinitialization config.

Every public item needs rustdoc. Keep types and behavior in focused modules,
with tests in neighboring `*_tests.rs` files. Cover wire representations and real
in-memory TinyBus calls. No placeholders, ignored tests, or lint exemptions.

Run from repository root:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
```

PRs go to the canonical upstream repository and should be ready for review.

## Tests live in `*_tests.rs` files

- Unit tests are never inline. Do not write a `#[cfg(test)] mod tests { ... }`
  block in a source file. Put the tests in a sibling `<module>_tests.rs`
  (`mod_tests.rs` beside a `mod.rs`, `lib_tests.rs` beside `lib.rs`) and declare
  it at the bottom of the module:

  ```rust
  #[cfg(test)]
  #[path = "foo_tests.rs"]
  mod tests;
  ```

- The test file starts with `use super::*;` and carries no `#[cfg(test)]` of its
  own. It is still a child module, so it reaches private items exactly as an
  inline module did.
- Name test files `<module>_tests.rs`; a second group for the same module is
  `<module>_<topic>_tests.rs`. Never `test.rs`, `tests.rs` or `<module>_test.rs`.
- Integration tests stay in the crate's `tests/` directory.
- OpenHuman's `scripts/externalize-inline-tests.mjs <repo-root> --write` moves
  inline test modules out mechanically; without `--write` it only reports.
- Existing `test.rs` and `<module>_test.rs` files predate this rule. Rename each
  to `<module>_tests.rs` (keep its `mod` name, add the `#[path]` attribute) the
  next time you touch it.
