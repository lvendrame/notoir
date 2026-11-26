param([switch]$VerifyOnly)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/windows-sdk.ps1"

if (-not $VerifyOnly) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $installation) { throw 'Visual Studio x64 C++ build tools were not found.' }
    $before = @{}
    Get-ChildItem Env: | ForEach-Object { $before[$_.Name] = $_.Value }
    Import-Module (Join-Path $installation 'Common7/Tools/Microsoft.VisualStudio.DevShell.dll')
    Enter-VsDevShell -VsInstallPath $installation -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64' | Out-Null
    Get-ChildItem Env: | Where-Object { $before[$_.Name] -cne $_.Value } | ForEach-Object {
        if ($_.Value -match '[\r\n]') { throw "Unexpected multiline developer environment variable: $($_.Name)" }
        "$($_.Name)=$($_.Value)" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
    }
    $linker = Join-Path $env:VCToolsInstallDir 'bin/Hostx64/x64/link.exe'
    if (-not (Test-Path -LiteralPath $linker)) { throw "Microsoft linker was not found: $linker" }
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $linker
    "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=$linker" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
}

$resolved = Get-NotoirWindowsLinker -Expected $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER
Write-Host "Verified Microsoft linker: $resolved"

if ($VerifyOnly) {
    $probe = Join-Path $env:RUNNER_TEMP 'notoir-linker-probe.rs'
    $exe = Join-Path $env:RUNNER_TEMP 'notoir-linker-probe.exe'
    [IO.File]::WriteAllText($probe, 'fn main() {}')
    & rustc --crate-name notoir_linker_probe -D warnings $probe -o $exe
    if ($LASTEXITCODE -ne 0) { throw 'Rust/MSVC linker smoke test failed.' }
    & $exe
    if ($LASTEXITCODE -ne 0) { throw 'Rust/MSVC smoke test executable failed.' }
}
