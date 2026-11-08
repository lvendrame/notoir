# Native MSIX acceptance on a disposable GitHub-hosted Windows runner only.
# Registers the unpacked development payload; never trusts a test certificate.
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/msix.ps1"
if (-not $IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Run package-install acceptance on a disposable GitHub-hosted Windows runner.'
}
Import-Module Appx -UseWindowsPowerShell
$identity = Get-NotoirStoreIdentity
if (Get-AppxPackage -Name $identity.Name) { throw 'A package with this identity is already installed; refusing to replace it.' }
$root = $script:NotoirStoreRoot
$out = Join-Path $root 'target/store-msix'
$manifest = Join-Path $out 'payload/AppxManifest.xml'
$profile = Join-Path $env:RUNNER_TEMP ([guid]::NewGuid().ToString())
$null = New-Item -ItemType Directory -Path (Join-Path $profile 'config') -Force
Set-Content -LiteralPath (Join-Path $profile 'config/settings.toml') -Value "[update]`nmode = 'off'"
$file = Join-Path $profile 'store test file.txt'
Set-Content -LiteralPath $file -Value 'original' -NoNewline
$alias = Join-Path $env:LOCALAPPDATA 'Microsoft/WindowsApps/notoir.exe'
$developerKey = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock'
$previous = Get-ItemProperty -Path $developerKey -Name AllowDevelopmentWithoutDevLicense -ErrorAction SilentlyContinue
$registered = $null
$gui = $null
$sequence = 0
$checks = [Collections.Generic.List[string]]::new()
function Assert-Store($Condition, [string]$Description) {
    if (-not $Condition) { throw "Store acceptance failed: $Description" }
    $script:checks.Add($Description)
    Write-Host "PASS: $Description"
}
function Start-StoreAlias {
    param([string[]]$Arguments)
    $start = [Diagnostics.ProcessStartInfo]::new($script:alias)
    $start.UseShellExecute = $false
    $start.WorkingDirectory = $script:profile
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $start.Environment['NOTOIR_HOME'] = $script:profile
    $start.Environment['NOTOIR_LANG'] = 'en'
    $start.Environment['NOTOIR_LOG'] = 'info'
    $start.Environment['NOTOIR_TEST_COMMANDS'] = '1'
    $process = [Diagnostics.Process]::Start($start)
    return @{ Process = $process; Out = $process.StandardOutput.ReadToEndAsync(); Error = $process.StandardError.ReadToEndAsync() }
}
function Send-StoreCommand {
    param([string]$Name, $Arguments)
    $request = Start-StoreAlias -Arguments @('--command', $Name, '--args', ($Arguments | ConvertTo-Json -Compress))
    if (-not $request.Process.WaitForExit(30000)) {
        $request.Process.Kill($true)
        throw "Store command timed out: $Name"
    }
    if ($request.Process.ExitCode -ne 0) { throw "Store command failed: $Name $($request.Error.GetAwaiter().GetResult())" }
}
function Get-StoreState {
    $script:sequence++
    $path = Join-Path $script:profile "state-$script:sequence.json"
    Send-StoreCommand 'test.ui' @{ op = 'snapshot'; output = $path }
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $path) {
            try { return Get-Content -LiteralPath $path -Raw | ConvertFrom-Json } catch {}
        }
        Start-Sleep -Milliseconds 100
    }
    throw 'Store app did not answer the native test bridge.'
}
function Wait-StoreState {
    param([scriptblock]$Ready)
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $state = Get-StoreState
        if (& $Ready $state) { return $state }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Store app state timed out: $($state | ConvertTo-Json -Depth 6 -Compress)"
}
try {
    $null = New-Item -Path $developerKey -Force
    $null = New-ItemProperty -Path $developerKey -Name AllowDevelopmentWithoutDevLicense -Value 1 -PropertyType DWord -Force
    Add-AppxPackage -Register $manifest
    $registered = Get-AppxPackage -Name $identity.Name
    Assert-Store ($registered.PackageFamilyName -ceq $identity.PackageFamilyName) 'Windows derives the reserved package family name'
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path -LiteralPath $alias) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    Assert-Store (Test-Path -LiteralPath $alias) 'the MSIX execution alias is registered'
    $version = Start-StoreAlias -Arguments @('--version')
    Assert-Store ($version.Process.WaitForExit(30000)) 'Store command-line version exits'
    Assert-Store ($version.Process.ExitCode -eq 0 -and $version.Out.GetAwaiter().GetResult().StartsWith('Notoir ')) 'execution alias preserves terminal output'
    $gui = Start-StoreAlias -Arguments @($file)
    # Wait for a native editor frame before forwarding commands into the host.
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $logs = @(Get-ChildItem -Path (Join-Path $profile 'logs/*') -ErrorAction SilentlyContinue |
            ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw }) -join "`n"
        if ($logs.Contains('test editor ready')) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    Assert-Store ($logs.Contains('test editor ready')) 'packaged GUI renders its first frame'
    $state = Wait-StoreState { param($s) @($s.documents | Where-Object { $_.text -ceq 'original' }).Count -eq 1 }
    Assert-Store ($state.distribution -eq 'microsoft-store') 'packaged executable uses Store-managed distribution policy'
    Send-StoreCommand 'editor.type' @{ text = 'edited ' }
    $null = Wait-StoreState { param($s) @($s.documents | Where-Object { $_.text -ceq 'edited original' }).Count -eq 1 }
    Send-StoreCommand 'file.saveAll' @{}
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ((Get-Content -LiteralPath $file -Raw) -cne 'edited original' -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    Assert-Store ((Get-Content -LiteralPath $file -Raw) -ceq 'edited original') 'packaged app opens, edits and saves a real file outside its package'
    Send-StoreCommand 'app.quit' @{}
    Assert-Store ($gui.Process.WaitForExit(30000)) 'packaged app quits normally'
    $gui.Out.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $out 'store-app-stdout.log')
    $gui.Error.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $out 'store-app-stderr.log')
    $checks | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $out 'native-acceptance.json') -Encoding utf8
} finally {
    if ($gui -and -not $gui.Process.HasExited) { $gui.Process.Kill($true); $gui.Process.WaitForExit() }
    if ($gui) {
        $gui.Out.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $out 'store-app-stdout.log')
        $gui.Error.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $out 'store-app-stderr.log')
    }
    if ($registered) { Remove-AppxPackage -Package $registered.PackageFullName }
    if ($previous) {
        $null = Set-ItemProperty -Path $developerKey -Name AllowDevelopmentWithoutDevLicense -Value $previous.AllowDevelopmentWithoutDevLicense
    } else { Remove-ItemProperty -Path $developerKey -Name AllowDevelopmentWithoutDevLicense -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $profile -Recurse -Force
}
