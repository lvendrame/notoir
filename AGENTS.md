Look at [CLAUDE.md](CLAUDE.md)

## Required verification before committing and pushing

- Read `rust-toolchain.toml` and the affected CI workflows. Use the same Rust/Cargo
  version locally and in CI; a stale local `stable` installation is not equivalent.
- Warnings and errors are unacceptable. Keep Cargo's `build.warnings = "deny"`
  and Clippy's `-D warnings` enabled. Fix diagnostics rather than suppressing them,
  filtering logs, or selecting an older compiler to avoid them. Check manifest,
  compiler, native-library, linker, and GitHub Action diagnostics.
- Run `cargo fmt --all --check`, `cargo deny check`,
  `cargo clippy --locked --workspace --all-targets --keep-going -- -D warnings`,
  and `cargo test --locked --workspace --no-fail-fast`. Read the full outputs and
  report ignored tests and any checks that could not run. `--no-run` only verifies
  compilation and must never be reported as an executed test suite.
- After native build or Windows environment changes, verify Microsoft `link.exe`
  is selected after all setup steps. Keep MSYS2 `/usr/bin` out of the global PATH;
  execute Unix build tools inside their own shell. Validate native builds without
  a cache as well as with the cache, and check action runtime deprecations.
- After a push, inspect every required job on the exact pushed SHA, including
  Linux/macOS/Windows Clippy and tests, formatting, deny, and Windows packaging
  checks. Investigate failures from their logs, fix them, and verify the new run.
  A green local run or a run from a different SHA does not establish CI success.
- State local and CI results separately. Do not claim completion or warning-free
  cross-platform validation while required jobs are pending, failed, or unverified.
