# Windows installer template

`windows-installer.nsi` is the cargo-packager **0.11.8** NSIS template, used under its MIT license
(see [license](cargo-packager-LICENSE-MIT)).

Upstream: https://github.com/crabnebula-dev/cargo-packager/blob/9e88b74ffb5531acf59c8bc371189a5e82b927d2/crates/packager/src/package/nsis/installer.nsi

The only behavior change is the uninstaller finalizer. It calls `notoir-sign.cmd "%1"` directly and requires
a zero exit status. The upstream template interpolates a Rust Debug-formatted command and does not enforce
the callback's exit status; our packaging preview showed that the uninstaller hook could fail silently.
The per-build signing receipts in `scripts/windows-signing.ps1` additionally require every expected hook
to have completed before artifacts can upload.

When upgrading cargo-packager, compare its current template with this copy, carry forward these changes,
and rerun the Windows packaging preview. Preserve the upstream license notice.
