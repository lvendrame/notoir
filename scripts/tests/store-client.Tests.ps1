$ErrorActionPreference = 'Stop'
$scriptPath = Join-Path $PSScriptRoot '../store-client.ps1'
$names = @('AZURE_AD_TENANT_ID', 'AZURE_AD_APPLICATION_CLIENT_ID', 'AZURE_AD_APPLICATION_SECRET', 'SELLER_ID', 'GITHUB_SHA', 'GITHUB_STEP_SUMMARY', 'LOCALAPPDATA')
$previous = @{}
foreach ($name in $names) {
    $previous[$name] = [Environment]::GetEnvironmentVariable($name)
    [Environment]::SetEnvironmentVariable($name, 'test-placeholder')
}
$global:storeCalls = [Collections.Generic.List[string]]::new()
$global:storeFailure = ''
$temp = Join-Path ([IO.Path]::GetTempPath()) "store-client-test-$([guid]::NewGuid())"
$null = New-Item -ItemType Directory -Path $temp
$env:LOCALAPPDATA = $temp
function global:msstore {
    $global:storeCalls.Add(($args -join ' '))
    $global:LASTEXITCODE = if ($args[0] -eq $global:storeFailure) { 1 } else { 0 }
}
function global:cmdkey.exe {
    if ($args[0] -ne '/delete:MicrosoftStoreCli:user=test-placeholder') { throw 'Unexpected credential deletion target.' }
    $global:storeCalls.Add('cleanup')
    $global:LASTEXITCODE = 0
}
function Assert($Condition, $Message) {
    if (-not $Condition) { throw $Message }
}
try {
    & $scriptPath -Mode VerifyAccess
    Assert ($global:storeCalls.Count -eq 3) 'VerifyAccess must configure, read and reset.'
    Assert ($global:storeCalls[1] -eq 'apps get 9P1JPTK7G5PX') 'VerifyAccess must read the correct product.'
    Assert ($global:storeCalls[2] -eq 'cleanup') 'VerifyAccess must remove credentials.'
    foreach ($failure in @('reconfigure', 'apps')) {
        $global:storeCalls.Clear()
        $global:storeFailure = $failure
        $failed = $false
        try { & $scriptPath -Mode VerifyAccess } catch { $failed = $true }
        Assert $failed "CLI failure must propagate: $failure"
        Assert ($global:storeCalls[-1] -eq 'cleanup') 'Failure must still remove credentials.'
        Assert (-not ($global:storeCalls | Where-Object { $_ -like 'publish*' })) 'Access verification must never publish.'
    }
    $global:storeCalls.Clear()
    $env:AZURE_AD_APPLICATION_SECRET = ''
    $failed = $false
    try { & $scriptPath -Mode VerifyAccess } catch { $failed = $_ -like '*Missing GitHub Actions secret*' }
    Assert $failed 'Missing credentials must fail before invoking CLI.'
    Assert ($global:storeCalls.Count -eq 0) 'Missing secrets must not cause interactive configuration.'
    $env:AZURE_AD_APPLICATION_SECRET = 'test-placeholder'
    $env:GITHUB_SHA = 'candidate'
    $env:GITHUB_STEP_SUMMARY = Join-Path $temp 'summary.md'
    $global:storeFailure = ''
    $package = Join-Path $temp 'test.msix'
    Set-Content -LiteralPath $package -Value 'mock package; never uploaded'
    $info = @{
        StoreId = '9P1JPTK7G5PX'; SourceCommit = 'candidate'; Package = 'test.msix'
        PackageVersion = '1.1.12.0'; Sha256 = (Get-FileHash $package).Hash
    }
    $infoFile = Join-Path $temp 'submission-info.json'
    $info | ConvertTo-Json | Set-Content $infoFile
    & $scriptPath -Mode Publish -PackageDirectory $temp
    Assert ($global:storeCalls[2] -like 'publish *test.msix -id 9P1JPTK7G5PX') 'Publish must submit the validated artifact to the correct product.'
    Assert ($global:storeCalls[-1] -eq 'cleanup') 'Publish must remove credentials.'
    foreach ($property in @('StoreId', 'SourceCommit', 'Sha256', 'Package')) {
        $global:storeCalls.Clear()
        $bad = $info.Clone()
        $bad[$property] = 'incorrect'
        $bad | ConvertTo-Json | Set-Content $infoFile
        $failed = $false
        try { & $scriptPath -Mode Publish -PackageDirectory $temp } catch { $failed = $true }
        Assert $failed "Invalid $property must reject publication."
        Assert ($global:storeCalls.Count -eq 0) 'Bad artifacts must fail before authentication.'
    }
    Write-Host 'Store client access and failure checks passed.'
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name]) }
    Remove-Item Function:\msstore
    Remove-Item Function:\cmdkey.exe
    Remove-Variable storeCalls,storeFailure -Scope Global
    Remove-Item -LiteralPath $temp -Recurse -Force
    $global:LASTEXITCODE = 0
}
