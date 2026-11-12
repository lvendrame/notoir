# Release checklist

This checklist covers GitHub installer releases. For Microsoft Store MSIX submissions, use
the [Store submission procedure](microsoft-store.md), including its Store-specific package validation.
Successful stable GitHub releases also invoke that Store workflow automatically. It requires the
Store publishing secrets, a green manual CI run on the same commit, and fresh MSIX/native validation.
Store submission failure does not remove the already-published GitHub release; inspect Partner Center
before retrying. Preview runs and prereleases do not submit Store updates.

Follow this checklist before creating any `v*` tag or running the Release workflow with `publish=true`.
The release workflow currently publishes independently of CI: its version and signing checks do **not**
require a green CI run. The checks below are a maintainer gate, not an automatic workflow dependency.

Run commands from the repository root. Examples use Bash/zsh and the GitHub CLI authenticated for
`lvendrame/notoir`. Keep the candidate variables in the same terminal throughout the procedure.

## What the Release workflow publishes

The [Release workflow](../.github/workflows/release.yml) distributes both GitHub downloads and stable
Microsoft Store updates. A successful run is not necessarily a published release: its trigger determines
whether publication is enabled.

| Trigger | GitHub downloads | Microsoft Store |
|---|---|---|
| Push a stable tag, e.g. `v0.1.13` | Build installers and publish the release | Build, validate and submit an MSIX for certification after GitHub publication |
| Manual run with `publish=true`, stable Cargo version | Create the version tag and publish installers | Same automatic Store submission |
| Prerelease tag or `publish=true` with a prerelease Cargo version | Publish a GitHub prerelease with installers | Skipped |
| Manual run with `publish=false` (the default) | Actions artifacts only; no release or downloads on the Releases page | Skipped |

For a normal stable release, build the installers once through the publishing run. Do not start a
preview or separate Store package workflow as a prerequisite. The publishing run performs these steps:

1. Build Windows MSI/EXE, macOS DMG and Linux DEB/AppImage installers, plus debug symbols and checksums.
2. Sign and verify the update manifest, then publish the files to the GitHub Releases page.
3. Call [store.yml](../.github/workflows/store.yml) with `mode: publish`. It builds the separate Store
   edition, validates the MSIX and native activation, then runs `store-client.ps1 -Mode Publish` to submit
   the package to Microsoft Store for certification.

Check GitHub assets after publication. The Store stage follows GitHub publication and does not delay
the availability of GitHub downloads. A successful Store submission means the package was submitted
for certification; it does not establish that the update is already approved or available to Store users.
Check Partner Center for that status. Store failure leaves the GitHub release and its downloads intact.

## 1. Prepare the candidate

- [ ] Start from an up-to-date `main` with no unrelated local changes. Fetch remote branches and tags.
- [ ] Choose an unused version. Update `[workspace.package].version` in `Cargo.toml` and refresh the
  workspace package versions in `Cargo.lock` with `cargo update --workspace`. Review the lockfile diff.
- [ ] Include all intended fixes, tests, specification coverage, documentation and release-note changes.
- [ ] Resolve dependency advisories and yanked versions before proceeding. Update affected packages
  deliberately, for example `cargo update -p <package> --precise <patched-version>`, and commit the
  resulting lockfile. Use a current advisory database and registry metadata; an offline audit is not
  sufficient for release approval. Do not suppress a vulnerability or yanked-version error just to release.
- [ ] For advisories involving build output or environment variables, follow the advisory's cleanup
  instructions too: upgrading does not erase old CI caches or artifacts. Remove affected cached build
  output and assess whether exposed credentials need rotation. Never print secrets into release evidence.

## 2. Run local checks

Install `cargo-deny` if needed (`cargo install cargo-deny --locked`). Audit dependencies before building
the candidate, since an advisory can affect build scripts themselves:

```sh
cargo deny --all-features check
```

- [ ] The audit passes. The `--all-features` option belongs **before** `check`, matching the CI action.
  Dependency/license/advisory failures block the release.
- [ ] Regenerate `cargo run -p spec-check -- --write` when specifications or scenario tags change, and
  `cargo run -p notoir --example gen_docs` when command/settings/menu/MCP references change.

Then run the required code checks:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] All three code checks pass.
- [ ] Run the native crash suite in a graphical session. These tests are ignored by ordinary
  `cargo test --workspace`, so that command alone does not verify crash recovery.

macOS / Windows:

```sh
cargo test -p notoir --test crash -- --ignored --test-threads=1
```

Linux (install the system libraries and Xvfb listed in [CI](../.github/workflows/ci.yml)):

```sh
xvfb-run -a cargo test -p notoir --test crash -- --ignored --test-threads=1
```

The native tests launch and kill real app processes using disposable `NOTOIR_HOME` profiles. Run with
normal display, local-socket and filesystem access. A sandbox or missing-display failure is not a pass.
Tests must wait for the current process's first rendered frame, loaded documents, persisted tab ownership,
and recovery state. Preserve these assertions when fixing failures; longer fixed sleeps do not prove readiness.

- [ ] Smoke-test the changed behavior in the real app. For window/menu changes on macOS, use
  `scripts/live-windows.py` and `scripts/live-native-menus.py` after `cargo build -p notoir`.
  Record the platforms actually checked; local macOS success does not establish Windows/Linux success.

## 3. Commit and verify the exact candidate in CI

- [ ] Commit the complete candidate, including its version and lockfile, and push **only the branch**
  to `origin/main`. Do not push a release tag yet. Confirm the working tree is clean.

```sh
candidate_sha="$(git rev-parse HEAD)"
release_version="$(cargo pkgid -p notoir)"
release_version="${release_version##*[#@]}"
release_tag="v${release_version}"
git status --short
git rev-parse origin/main
```

- [ ] `origin/main` equals `candidate_sha`. Start a fresh manual CI run on this candidate:

```sh
gh workflow run ci.yml --repo lvendrame/notoir --ref main
gh run list --repo lvendrame/notoir --workflow ci.yml --event workflow_dispatch \
  --commit "$candidate_sha" --limit 5 --json databaseId,headSha,status,conclusion,url
```

Replace `REPLACE_WITH_NEW_RUN_ID` with the newly dispatched run ID from that output. Inspect and wait for it:

```sh
ci_run_id=REPLACE_WITH_NEW_RUN_ID
gh run view "$ci_run_id" --repo lvendrame/notoir --json headSha,event,status,conclusion,jobs
gh run watch "$ci_run_id" --repo lvendrame/notoir --exit-status
```

- [ ] Its `headSha` is exactly `candidate_sha`, and its event is `workflow_dispatch`.
- [ ] The run is **completed**, its conclusion is **success**, and all nine jobs below succeeded:

| Required job | Platform |
|---|---|
| `fmt` | Ubuntu |
| `deny` | Ubuntu |
| `Windows packaging checks` | Windows |
| `clippy + test` | Windows, macOS and Ubuntu (three jobs) |
| `crash suite` | Windows, macOS and Ubuntu (three jobs) |

A normal push/PR run skips the crash suite. A green push run, an older nightly run, a successful local
test run, or green test steps in a still-running job do not satisfy this gate. Wait for final job cleanup
and cache uploads too. Any failure, cancellation, skipped required job or candidate change blocks tagging.
Fix the cause, commit the fix, and repeat validation on the new SHA.

## 4. Check publication credentials

- [ ] The repository secret `MINISIGN_SECRET_KEY` and variable `NOTOIR_UPDATE_PUBKEY` are present and
  contain a matching minisign key pair. They are required for publication, including prereleases.
  See [signing setup](guide/src/updates.md#release-signing-setup). Check names without printing secrets:

```sh
gh secret list --repo lvendrame/notoir
gh variable list --repo lvendrame/notoir
```

`NOTOIR_UPDATE_URL` defaults to the latest release's `update.json`.

- [ ] Stable releases require the four [Store publishing secrets](microsoft-store.md#credentials-and-first-access-check).
  Store packaging, native validation and submission run automatically after GitHub publication.
- [ ] When `WINDOWS_SIGNING_PROVIDER` is unset or `none`, standalone Windows installers remain unsigned.
  If a provider is configured, verify the expected trusted publisher and timestamp after packaging,
  following [Windows signing setup](windows-signing.md).

Publish after candidate CI passes, then verify the generated packages in section 6. A packaging preview
(`release.yml` with `publish=false`) or separate Store package run (`store.yml` with `mode=package`) is
optional for diagnosis. Neither is a publication gate. A preview creates Actions artifacts only; it does
not create a downloadable GitHub release.

## 5. Create and push the release tag

- [ ] Record the candidate SHA, version, successful CI URL and smoke-test results
  in the release preparation notes. Re-run the dependency audit immediately before tagging, especially
  if validation was performed on an earlier day.
- [ ] After candidate CI and credential checks pass, fetch again, confirm the candidate has not changed and the tag is
  unused, then tag the recorded SHA and push that tag only. The `&&` chain stops on the first failure:

```sh
git fetch origin --tags &&
test -z "$(git status --porcelain)" &&
test "$(git rev-parse HEAD)" = "$candidate_sha" &&
test "$(git rev-parse origin/main)" = "$candidate_sha" &&
test -z "$(git tag -l "$release_tag")" &&
cargo deny --all-features check &&
git tag -a "$release_tag" "$candidate_sha" -m "Notoir $release_version" &&
git push origin "refs/tags/$release_tag"
```

The tag must be `v<Cargo version>`. Prerelease suffixes must be numeric for MSI compatibility, e.g.
`v0.2.0-1`. Alternatively, the Release workflow's `publish=true` option creates the tag itself; it must
target the same validated candidate and satisfy every gate above. Choose one publication method.

Never move or force-push a published release tag. Source or dependency fixes after publication require
a new version and the same checklist.

## 6. Verify the published release

- [ ] Wait for the tag-triggered Release run to complete successfully. Confirm it built `candidate_sha`.
  A pushed tag or a running workflow is not yet a published release.
- [ ] Check the GitHub release includes all installers, debug symbols, `SHA256SUMS.txt`, `update.json`
  and `update.json.minisig`, and that their versions match the tag.
- [ ] Confirm the publishing job verified the minisign signature against `NOTOIR_UPDATE_PUBKEY`.
  Check the manifest has `macos` and `linux-appimage` artifacts with the correct download URLs and
  checksums, plus `windows` for the standalone MSI. Store updates are managed
  by Microsoft. Smoke-test downloading and checking for updates from an installed app.
- [ ] Publish release notes describing the changes and any installer/signing limitations. Retain the
  candidate SHA, CI run URL, release run URL and verification results for the next maintainer.

- [ ] Download the published installers and debug symbols. Require the versioned macOS universal DMG,
  Windows MSI and NSIS installer, Linux DEB and AppImage, plus per-OS checksums. Verify each checksum
  from its artifact directory (`sha256sum -c` on Linux or `shasum -a 256 -c` on macOS).
- [ ] Install and launch packages on available test systems and check the changed flows. Record the
  packages/platforms exercised and any limitations. Without a Windows signing provider, verify the
  unsigned signing-hook receipts; with a provider, verify publisher and timestamp on the application,
  installers and NSIS uninstaller.
- [ ] Check the automatic Microsoft Store package/native validation and submission after GitHub
  publication. A Store failure does not remove or block the GitHub release. Inspect Partner Center
  before retrying a submission.
- [ ] If generated packages fail verification, stop recommending the affected downloads, report the
  failure and prepare a fixed version. Never move or force-push a published tag.

## Why candidate CI remains required

The November 2026 releases exposed dependency advisory/yank failures and ignored native crash tests
that ordinary workspace tests did not detect. A fresh dependency audit and exact-commit manual CI run,
including all crash jobs, precede tagging. Installer generation and Store validation are checked after
publication; separate preview workflows are optional.
