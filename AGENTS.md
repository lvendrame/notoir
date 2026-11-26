Look at [CLAUDE.md](CLAUDE.md)

## Required verification before committing and pushing

### Strict local verification gate

**Pushing is strictly forbidden until every required local verification has
completed successfully on the final changes, with zero warnings and errors.**

- CI is a final confirmation, never a debugging environment or a substitute for
  local reproduction and verification. Do not push speculative fixes, create
  verification-only commits on the remote, or trigger/rerun CI to discover whether
  an unverified change works.
- If a required check is unavailable locally, including native Windows validation,
  stop and report the missing verification. An unavailable, pending, skipped, or
  failed check does not satisfy the gate. Do not push to work around that gap.
- A previous or general instruction to "commit and push" does not waive this gate
  or authorize additional pushes and CI runs for debugging.
- Before any push, present the exact tool versions, commands, exit results,
  reviewed warnings/errors, existing ignored tests, and platform-specific results
  for the final changes. Keep the logs available for review. After a change made
  during verification, rerun the affected checks before considering the gate met.
- Test build/environment helpers locally with representative fixtures and failure
  cases before pushing. Syntax checks alone do not establish correct behavior;
  cover PATH collisions, multiple command matches, and checker version differences
  when relevant.

### Required checks

- Read `rust-toolchain.toml` and the affected CI workflows. Use the same Rust/Cargo
  version locally and in CI; a stale local `stable` installation is not equivalent.
  Match auxiliary checker versions too, and inspect successful job logs: zero exit
  status alone does not prove the absence of errors or warnings.
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
  checks. Investigate failures from their logs, reproduce and fix them locally,
  and repeat the local verification gate before any further push or CI run.
  A green local run or a run from a different SHA does not establish CI success.
- State local and CI results separately. Do not claim completion or warning-free
  cross-platform validation while required jobs are pending, failed, or unverified.
