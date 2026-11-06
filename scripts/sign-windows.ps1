param([string]$Path, [switch]$ValidateOnly, [switch]$VerifyOnly)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/windows-signing.ps1"
$required = $env:WINDOWS_SIGNING_REQUIRED -eq 'true'
$provider = Assert-NotoirWindowsSigningConfiguration -Required:$required
if ($ValidateOnly) { return }
if ([string]::IsNullOrWhiteSpace($Path) -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "Missing file for Windows signing: $Path"
}
if ($provider -eq 'none') {
    Write-Host "Unsigned Windows preview (no signing identity configured): $Path"
    return
}
if ($VerifyOnly) { Test-NotoirWindowsSignature -Path $Path }
else { Invoke-NotoirWindowsSign -Path $Path }
