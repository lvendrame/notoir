# Microsoft Store submission

This is the MSIX route for the product already reserved in Partner Center. Microsoft signs the package
after certification; no Azure Artifact Signing account, PFX certificate or minisign key is required for
this workflow. Direct-download MSI/EXE releases use the separate [Authenticode setup](windows-signing.md).

## Reserved identity

The public identity is stored in [identity.json](../packaging/windows-store/identity.json):

| Field | Value |
|---|---|
| Package name | `LusFernandoSaquetimVendra.Notoir` |
| Publisher | `CN=EC133CA5-BEED-40F2-8541-21EEF52E4CF7` |
| Publisher display name | Luís Fernando Saquetim Vendrame |
| Package family | `LusFernandoSaquetimVendra.Notoir_73zmgdq8ngj54` |
| Store ID | `9P1JPTK7G5PX` |

Keep these values exactly as assigned by Partner Center. The package SID is not needed to build or
submit this app. The Store ID identifies the listing; it is not a signing credential.

## Build the upload package

1. Push the complete candidate to `main` and run the appropriate CI checks.
2. In GitHub, open **Actions → Microsoft Store package → Run workflow** and select the candidate branch.
   CLI equivalent:

   ```sh
   gh workflow run store.yml --repo lvendrame/notoir --ref main
   ```

3. Wait for **Build and validate Store MSIX** to complete successfully on the intended commit.
4. Download its **notoir-microsoft-store-x64** artifact and extract the ZIP.
5. The upload file is **`notoir_1.1.11.0_x64_store.msix`** for Cargo version `0.1.11`.
   `submission-info.json` records the identity, source/package versions, candidate SHA and SHA-256.
   Check the downloaded package against `SHA256SUMS.txt` before uploading.

The workflow validates the manifest/assets with the Windows SDK, builds the `microsoft-store` edition,
checks its x64 PE header and runtime imports, and runs MakeAppx with semantic validation enabled.
It registers the unpacked development payload on a disposable Windows runner and verifies the package
family, CLI alias, GUI activation, and a real file open/edit/save/quit cycle. No test certificate is trusted.
The runner restores its developer-mode setting and unregisters the test package afterward.

This package is deliberately unsigned for **Partner Center submission**. It is not a ready-to-install
direct-download MSIX. Successful SDK and native checks do not replace Microsoft's certification.

### Store version numbering

Microsoft requires a nonzero major version and reserves the fourth component. The deterministic mapping
is `(Cargo major + 1).minor.patch.0`:

| App version | MSIX version |
|---|---|
| `0.1.11` | `1.1.11.0` |
| `0.1.12` | `1.1.12.0` |
| `1.0.0` | `2.0.0.0` |

The app still reports its Cargo version. Do not reset this offset for a later major release; that could
downgrade the Store package. Prerelease/build-metadata versions are rejected. Each newly published
package must have a greater version than the previous Store package.

## What to click in Partner Center

1. Open **Notoir → Product release → Start submission** (or open the existing draft).
2. Open **Packages**, upload the **`.msix`** file, and wait for package validation. Do not upload the
   GitHub artifact ZIP, `SHA256SUMS.txt`, or an MSI/EXE installer into this MSIX product.
3. Check that the detected identity/version matches `submission-info.json`, then save the Packages section.
4. Complete **Pricing and availability**, **Properties**, **Age ratings** and **Store listings**.
   Add the description, screenshots, logo, support details and a privacy-policy URL describing actual data use.
   The package includes all 14 application languages; the first Store listing can be completed in English.
5. If Partner Center asks about the restricted **`runFullTrust`** capability, explain why the classic desktop
   application needs it. Suggested certification notes:

   > Notoir is a native Win32 text editor built with Rust and egui. Full trust is required for user-selected
   > file/folder access, file watching, native dialogs, Git integration, optional user-installed Lua plugins,
   > and the optional authenticated local MCP server. It runs as the current user. Store package updates,
   > file associations and command aliases are managed by Windows/Microsoft Store; the standalone updater
   > and manual registration commands are disabled in this edition. No startup service is installed.

   Describe any enabled network features accurately in the listing/privacy policy. Git, URL downloads,
   plugins and MCP can communicate with endpoints or clients selected/configured by the user.
6. Review the remaining required sections and choose **Submit for certification** when ready.
7. After approval/publication, Microsoft supplies the signed Store distribution. The listing URL will be
   `https://apps.microsoft.com/detail/9P1JPTK7G5PX`; it is not expected to resolve while the product is a draft.

Uploading a package does not itself publish the app. Complete the account verification, listing and
certification steps requested by Partner Center. Test the Store-distributed installation on a clean
Windows 10/11 system after certification, including Open With, the `notoir` alias and Store updates.

## Store-edition behavior

- Microsoft Store owns updates. The standalone update-check/download/install commands and `update.*`
  controls are unavailable in this build, even if a standalone updater configuration is supplied.
- MSIX registers the file types and the `notoir.exe` execution alias. Manual Help commands that write
  standalone PATH/context-menu registrations are unavailable; Windows removes package registrations on uninstall.
- Store and MSI/EXE processes use separate instance channels, so a Store launch cannot silently forward
  into a standalone process with a different updater. The existing application directory identifier is unchanged.
- The native payload statically links the MSVC runtime, and packaging rejects dynamic VC runtime imports.
  No external VC runtime package or paid signing service is needed for this submission.

## Local Windows builds and checks

Install the Rust MSVC toolchain, Visual Studio C++ tools, PowerShell 7 and a Windows 10/11 SDK containing
MakeAppx. From the repository root:

```powershell
pwsh -NoProfile -File scripts/tests/msix.Tests.ps1
cargo test --locked -p notoir --features microsoft-store --test microsoft_store
pwsh -NoProfile -File scripts/package-store.ps1
```

The builder always compiles with `--features microsoft-store` and static CRT flags before packaging;
it does not offer a shortcut that could accidentally package a standalone-updater executable.
Generated output is under `target/store-msix/`. `test-store-install.ps1` is guarded for disposable
GitHub-hosted runners and should not be used to replace an installed user package.

For Store submission, record a green manual CI run (all platform/crash jobs plus Windows packaging checks)
and a green Store-package workflow on the exact candidate SHA. The standalone GitHub-installer packaging
preview is required separately when also making a direct-download release.

## Microsoft references

- [MSIX submission requirements and Store signing](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements)
- [Package version rules](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/package-version-numbering)
- [Manual Win32 MSIX packaging](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion)
