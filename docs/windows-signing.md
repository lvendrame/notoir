# Windows signing setup (issue #1)

For the **Microsoft Store MSIX** distribution, follow [Store submission](microsoft-store.md) instead.
Microsoft signs those packages after certification. The Azure/PFX setup below applies to direct-download
MSI/EXE installers and is not needed for the reserved MSIX product.

[Issue #1](https://github.com/lvendrame/notoir/issues/1) reports SmartScreen displaying **Unknown publisher**
for an unsigned MSI. Fixing the publisher identity requires a publicly trusted Authenticode signing
identity. Changing installer metadata or signing `update.json` with minisign does not sign Windows code.

The repository integration is prepared; identity validation, signing credentials and a new signed release
are still required before this issue can be considered resolved. Existing downloaded installers do not
change when the pipeline is updated.

Signing displays the verified publisher and allows reputation to accumulate across releases. It does
**not** guarantee that every SmartScreen prompt immediately disappears. Microsoft also evaluates download
reputation; self-signed certificates do not establish public trust. See Microsoft's
[SmartScreen guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).

## Choose a signing identity

The pipeline supports two providers:

| Provider | Use |
|---|---|
| `azure` | Azure Artifact Signing (formerly Trusted Signing), using GitHub OIDC and a Public Trust certificate profile. The private signing key remains in the service. |
| `pfx` | An existing compatible, publicly trusted Authenticode certificate whose private key is available as a password-protected PFX. |

Azure is the prepared cloud path. Check current eligibility and pricing before purchasing or creating
resources. Microsoft currently distinguishes organization and individual eligibility; an individual's
location may be unsupported even when organizations in that location are eligible. New public CA
certificates may require hardware-backed keys and may not be exportable as PFX. Confirm CI compatibility
with the provider rather than buying a certificate solely for the PFX option. Microsoft's
[signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
also describe Microsoft Store distribution as an alternative.

## Azure Artifact Signing

1. Follow the [Azure quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart) to
   create a signing account, complete identity validation and create a **Public Trust** certificate profile.
   A test or Private Trust profile does not establish publisher trust for general Windows users.
2. Register a dedicated Microsoft Entra application/service principal for GitHub signing. Record its
   client ID, tenant ID and the signing account's subscription ID.
3. Assign that principal the **Artifact Signing Certificate Profile Signer** role on the chosen
   certificate profile. Follow [Microsoft's role setup](https://learn.microsoft.com/en-us/azure/artifact-signing/tutorial-assign-roles)
   and scope the signing permission to that profile rather than the subscription.
4. In GitHub repository settings, create the environment **`windows-signing`**. Restrict deployment refs
   to `main` for packaging previews and `v*` tags for publication. Configure reviewer requirements according
   to your release policy; the workflow itself does not require a manual approval step.
5. Add an Entra federated identity credential for this GitHub environment:

   | Field | Value |
   |---|---|
   | Issuer | `https://token.actions.githubusercontent.com` |
   | Audience | `api://AzureADTokenExchange` |
   | Subject | `repo:lvendrame/notoir:environment:windows-signing` |

   The subject must match the repository/environment exactly. Forks or an environment with another name
   must not receive this identity's signing permission. See the
   [GitHub OIDC setup](https://learn.microsoft.com/en-us/azure/developer/github/connect-from-azure-openid-connect).
6. Set these **repository variables** (Settings → Secrets and variables → Actions → Variables):

   | Variable | Value |
   |---|---|
   | `WINDOWS_SIGNING_PROVIDER` | `azure` |
   | `WINDOWS_EXPECTED_PUBLISHER` | Exact certificate publisher/common name from the validated identity; this may be your legal name rather than the Notoir brand |
   | `AZURE_CLIENT_ID` | Entra application client ID |
   | `AZURE_TENANT_ID` | Entra tenant ID |
   | `AZURE_SUBSCRIPTION_ID` | Signing account subscription ID |
   | `AZURE_SIGNING_ENDPOINT` | Regional HTTPS endpoint, e.g. `https://weu.codesigning.azure.net/`; must match the account's region |
   | `AZURE_SIGNING_ACCOUNT` | Signing account name |
   | `AZURE_SIGNING_PROFILE` | Public Trust certificate profile name |

   Use repository variables because the early release validation job runs outside the signing environment.
   These identifiers are not private keys. No Azure client secret is needed: the Windows job authenticates
   with OIDC via `azure/login`, and the signing hook uses that Azure CLI session only.

Only the Windows packaging job has `id-token: write` and uses `windows-signing`. Unix packaging jobs
do not receive that signing identity. The Windows runner installs the pinned Microsoft `ArtifactSigning`
PowerShell module; the version is defined in `scripts/windows-signing.ps1` and should be reviewed when updated.
Signing-tool downloads are not added to the Rust build cache.

## Existing PFX certificate

For an existing compatible exportable certificate:

- Set repository variables `WINDOWS_SIGNING_PROVIDER=pfx` and `WINDOWS_EXPECTED_PUBLISHER` to the
  certificate's exact publisher/common name.
- Store the base64-encoded PFX as repository secret `WINDOWS_CERTIFICATE`, and its nonempty password as
  `WINDOWS_CERTIFICATE_PASSWORD`. Upload secrets through GitHub's secure settings or `gh secret set`
  using file/stdin input; never put the private key or password in a command example, commit or log.
- The workflow writes the PFX into runner temporary storage and removes it in an `always()` cleanup step.
  `signtool.exe` is located from the Windows SDK, even when it is absent from `PATH`.

For local Windows packaging, install PowerShell 7, the Windows SDK and cargo-packager 0.11.8, then supply
the same provider/publisher variables plus `WINDOWS_CERTIFICATE_PATH` and `WINDOWS_CERTIFICATE_PASSWORD`
through your credential manager/environment. Run `scripts/package.sh` from Git Bash. Keep private signing
material outside the repository and build caches. A self-signed test certificate is not a public-release fix.

## What the pipeline verifies

Before building a published release, configuration validation rejects missing/unknown providers,
incomplete credentials and a missing expected publisher. A tag or `publish=true` cannot publish unsigned
Windows packages. A manual `publish=false` preview may remain unsigned when the provider is empty or `none`.
An explicitly configured provider must be valid even for a preview; signing failures are never ignored.

The cargo-packager hook invokes `scripts/notoir-sign.cmd` from `PATH`, which locates its PowerShell script
independently of the current working directory. This covers:

- `notoir.exe` **before** it is embedded in the MSI and NSIS installer;
- the MSI and NSIS setup executable;
- the NSIS-generated uninstaller.

Each signature uses SHA-256 and an RFC 3161 timestamp. The hook rejects untrusted/invalid signatures,
missing timestamps and a publisher different from `WINDOWS_EXPECTED_PUBLISHER`. The application and final
installers are checked again before checksums are calculated and artifacts uploaded. Azure's short-lived
certificates make timestamping particularly important; see the
[signing integration documentation](https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-signing-integrations).

NSIS can ignore an uninstaller signing command's failure, so packaging also requires fresh per-build
receipts for all four hook targets. A missing uninstaller receipt blocks upload even if NSIS produced
an outer installer. Unsigned preview receipts cannot satisfy signed-release validation, and changed
application/installer bytes invalidate their receipts before checksums are generated.
The pinned [NSIS template](../scripts/templates/README.md) invokes the batch hook directly and enforces its
exit status; this avoids the upstream template's command-formatting problem exposed by the preview.

The existing minisign updater key is still required separately. It authenticates update metadata; it
cannot replace Authenticode or provide the Windows publisher identity.

## Validate before the next release

1. Run the offline regression checks on Windows:

   ```powershell
   pwsh -NoProfile -File scripts/tests/windows-signing.Tests.ps1
   ```

   CI runs these as **Windows signing checks** without production credentials or trust-store changes.
   They check configuration rejection, signature/timestamp/publisher failures, provider calls and the
   real batch entry point. Mocked provider tests do not establish that a real certificate is trusted.
2. After the external identity and variables are configured, follow the [release checklist](releasing.md).
   Dispatch the Release workflow on the candidate with **publish=false**. Confirm the logs actually
   sign and verify Windows files; an unsigned preview is not sufficient evidence for issue #1.
3. Download the preview artifacts and verify the MSI, setup executable and installed `notoir.exe` on a
   clean Windows 10/11 test system:

   ```powershell
   $signature = Get-AuthenticodeSignature -LiteralPath '.\notoir_<version>_x64_en-US.msi'
   $signature | Format-List Status, StatusMessage, SignerCertificate, TimeStamperCertificate
   ```

   Require `Status: Valid`, the expected publisher and a timestamp. Inspect the NSIS uninstaller after
   installation too. Exercise install, launch, uninstall and the MSI update path. Keep SmartScreen enabled;
   record any remaining reputation warning separately from signature validation.
4. Publish a **new version** only after the candidate's required CI and signed packaging preview pass.
   Do not replace historical release installers under the same tag: signatures change the bytes and hashes.
   Close issue #1 only after a signed public release and Windows verification, not just after this code lands.

If setup is incomplete, use `publish=false` to test unsigned packaging. Missing external signing credentials
are an intentional publication blocker until a trusted identity is ready.
