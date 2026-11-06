# Offline regression tests: no public certificates, network calls or trust-store changes.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
. "$PSScriptRoot/../windows-signing.ps1"
$fixture = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
$null = New-Item -ItemType Directory -Path $fixture
$file = Join-Path $fixture 'app with spaces.exe'
Set-Content -LiteralPath $file -Value 'fixture'
$passed = 0

function Assert-True($Condition, [string]$Description) {
    if (-not $Condition) { throw "FAIL: $Description" }
    $script:passed++
    Write-Host "PASS: $Description"
}
function Assert-Throws([scriptblock]$Action, [string]$Expected) {
    $message = $null
    try { & $Action | Out-Null } catch { $message = $_.Exception.Message }
    Assert-True ($null -ne $message -and $message.Contains($Expected)) "rejects $Expected"
}

# Fixtures exercise the decision logic; Windows itself supplies real WinTrust
# verification in the release hook. This mock never changes machine trust.
function Get-AuthenticodeSignature {
    param([string]$LiteralPath)
    Assert-True ($LiteralPath -eq $script:file) 'signature path preserves spaces'
    return $script:signature
}
$certificate = [pscustomobject]@{ Publisher = 'Notoir Test Publisher' }
$certificate | Add-Member -MemberType ScriptMethod -Name GetNameInfo -Value { param($Type, $Issuer) $this.Publisher }

try {
    $names = @('WINDOWS_SIGNING_PROVIDER', 'WINDOWS_SIGNING_REQUIRED', 'WINDOWS_EXPECTED_PUBLISHER',
        'WINDOWS_SIGNING_RECEIPTS', 'WINDOWS_SIGNING_PHASE',
        'WINDOWS_CERTIFICATE_PATH', 'WINDOWS_CERTIFICATE_PASSWORD', 'AZURE_CLIENT_ID', 'AZURE_TENANT_ID',
        'AZURE_SUBSCRIPTION_ID', 'AZURE_SIGNING_ENDPOINT', 'AZURE_SIGNING_ACCOUNT', 'AZURE_SIGNING_PROFILE')
    $previous = @{}
    foreach ($name in $names) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name)
        [Environment]::SetEnvironmentVariable($name, $null)
    }
    Assert-True ((Assert-NotoirWindowsSigningConfiguration) -eq 'none') 'unsigned previews are explicit'
    Assert-Throws { Assert-NotoirWindowsSigningConfiguration -Required } 'publication requires Authenticode'
    $env:WINDOWS_SIGNING_PROVIDER = 'typo'
    Assert-Throws { Assert-NotoirWindowsSigningConfiguration } 'must be azure, pfx or none'
    $env:WINDOWS_SIGNING_PROVIDER = 'pfx'
    Assert-Throws { Assert-NotoirWindowsSigningConfiguration } 'WINDOWS_EXPECTED_PUBLISHER'
    $env:WINDOWS_EXPECTED_PUBLISHER = 'Notoir Test Publisher'
    Assert-Throws { Assert-NotoirWindowsSigningConfiguration } 'PFX signing requires'
    Assert-True ((Assert-NotoirWindowsSigningConfiguration -Required -CertificateAvailable:$true -PasswordAvailable:$true) -eq 'pfx') 'preflight checks secret availability without reading secret values'
    Assert-Throws { Assert-NotoirWindowsSigningConfiguration -CertificateAvailable:$true -PasswordAvailable:$false } 'PFX signing requires'

    $env:WINDOWS_SIGNING_PROVIDER = 'azure'
    Assert-Throws { Assert-NotoirWindowsSigningConfiguration } 'AZURE_CLIENT_ID'
    foreach ($name in @('AZURE_CLIENT_ID', 'AZURE_TENANT_ID', 'AZURE_SUBSCRIPTION_ID', 'AZURE_SIGNING_ACCOUNT', 'AZURE_SIGNING_PROFILE')) {
        [Environment]::SetEnvironmentVariable($name, 'fixture')
    }
    $env:AZURE_SIGNING_ENDPOINT = 'http://weu.codesigning.azure.net/'
    Assert-Throws { Assert-NotoirWindowsSigningConfiguration } 'HTTPS regional'
    $env:AZURE_SIGNING_ENDPOINT = 'https://weu.codesigning.azure.net.attacker.invalid/'
    Assert-Throws { Assert-NotoirWindowsSigningConfiguration } 'HTTPS regional'
    $env:AZURE_SIGNING_ENDPOINT = 'https://weu.codesigning.azure.net/'
    Assert-True ((Assert-NotoirWindowsSigningConfiguration -Required) -eq 'azure') 'complete Azure configuration is accepted'

    $script:signature = [pscustomobject]@{ Status = 'NotSigned'; SignerCertificate = $null; TimeStamperCertificate = $null }
    Assert-Throws { Test-NotoirWindowsSignature -Path $file } 'Invalid Authenticode'
    $script:signature.Status = 'NotTrusted'
    $script:signature.SignerCertificate = $certificate
    Assert-Throws { Test-NotoirWindowsSignature -Path $file } 'Invalid Authenticode'
    $script:signature.Status = 'Valid'
    Assert-Throws { Test-NotoirWindowsSignature -Path $file } 'Missing Authenticode timestamp'
    $script:signature.TimeStamperCertificate = $certificate
    $env:WINDOWS_EXPECTED_PUBLISHER = 'Different Publisher'
    Assert-Throws { Test-NotoirWindowsSignature -Path $file } 'Unexpected Authenticode publisher'
    $env:WINDOWS_EXPECTED_PUBLISHER = 'Notoir Test Publisher'
    Test-NotoirWindowsSignature -Path $file
    Assert-Throws { Test-NotoirWindowsSignature -Path (Join-Path $fixture 'missing.exe') } 'Missing signing output'

    # Capture provider calls instead of contacting Azure or invoking SignTool.
    function Import-Module { param($Name, $RequiredVersion, $ErrorAction) }
    function Invoke-ArtifactSigning {
        param($Endpoint, $CodeSigningAccountName, $CertificateProfileName, $Files,
            $FileDigest, $TimestampRfc3161, $TimestampDigest, $ExcludeEnvironmentCredential,
            $ExcludeWorkloadIdentityCredential, $ExcludeManagedIdentityCredential,
            $ExcludeSharedTokenCacheCredential, $ExcludeVisualStudioCredential,
            $ExcludeVisualStudioCodeCredential, $ExcludeAzureCliCredential,
            $ExcludeAzurePowerShellCredential, $ExcludeAzureDeveloperCliCredential,
            $ExcludeInteractiveBrowserCredential)
        $script:azureArguments = $PSBoundParameters
    }
    Invoke-NotoirWindowsSign -Path $file
    $arguments = $script:azureArguments
    foreach ($pair in @(@('Files', $file), @('FileDigest', 'SHA256'), @('TimestampDigest', 'SHA256'),
                        @('TimestampRfc3161', 'http://timestamp.acs.microsoft.com'),
                        @('ExcludeInteractiveBrowserCredential', $true), @('ExcludeAzureCliCredential', $false))) {
        Assert-True ($arguments.ContainsKey($pair[0]) -and $arguments[$pair[0]] -eq $pair[1]) "Azure signing passes $($pair[0])"
    }
    $env:WINDOWS_SIGNING_PROVIDER = 'pfx'
    $env:WINDOWS_CERTIFICATE_PATH = $file
    $env:WINDOWS_CERTIFICATE_PASSWORD = 'test-only-password'
    function Get-NotoirSignTool { return 'Invoke-FakeSignTool' }
    function Invoke-FakeSignTool { $script:pfxArguments = $args; $global:LASTEXITCODE = $script:signExit }
    $script:signExit = 1
    Assert-Throws { Invoke-NotoirWindowsSign -Path $file } 'SignTool failed'
    $script:signExit = 0
    Invoke-NotoirWindowsSign -Path $file
    Assert-True ($script:pfxArguments[-1] -eq $file) 'SignTool receives the full file path as one argument'
    Assert-True ($script:pfxArguments -contains '/fd' -and $script:pfxArguments -contains 'SHA256' -and $script:pfxArguments -contains '/tr') 'PFX signing requests SHA256 and RFC3161 timestamping'

    $env:WINDOWS_SIGNING_PROVIDER = 'none'
    $env:WINDOWS_SIGNING_PHASE = 'nsis'
    $env:WINDOWS_SIGNING_RECEIPTS = Join-Path $fixture 'receipts'
    $null = New-Item -ItemType Directory -Path $env:WINDOWS_SIGNING_RECEIPTS
    foreach ($name in @('notoir.exe', 'build.msi', 'release-setup.exe', 'temporary-uninstaller.exe')) {
        $path = Join-Path $fixture $name
        Set-Content -LiteralPath $path -Value 'packaging fixture'
        Write-NotoirWindowsSigningReceipt -Path $path -Signed $false
    }
    Remove-Item -LiteralPath (Join-Path $fixture 'temporary-uninstaller.exe')
    Test-NotoirWindowsSigningReceipts
    Assert-True $true 'all four hook receipts are required even when NSIS removes its temporary uninstaller'
    $uninstallerReceipt = Join-Path $env:WINDOWS_SIGNING_RECEIPTS 'uninstaller.json'
    $savedReceipt = Get-Content -LiteralPath $uninstallerReceipt -Raw
    Remove-Item -LiteralPath $uninstallerReceipt
    Assert-Throws { Test-NotoirWindowsSigningReceipts } 'Missing uninstaller signing receipt'
    Set-Content -LiteralPath $uninstallerReceipt -Value $savedReceipt
    Set-Content -LiteralPath (Join-Path $fixture 'build.msi') -Value 'tampered after signing'
    Assert-Throws { Test-NotoirWindowsSigningReceipts } 'Changed signing output'
    $env:WINDOWS_SIGNING_PROVIDER = 'pfx'
    Assert-Throws { Test-NotoirWindowsSigningReceipts } 'does not match the configured signing identity'

    # Exercise the actual batch entry point on Windows, including exit-code
    # propagation and a working directory different from the repository.
    if ($IsWindows) {
        $env:WINDOWS_SIGNING_PROVIDER = 'none'
        $env:WINDOWS_SIGNING_REQUIRED = 'false'
        $wrapper = (Resolve-Path "$PSScriptRoot/../notoir-sign.cmd").Path
        Push-Location $fixture
        try {
            & $wrapper $file
            Assert-True ($LASTEXITCODE -eq 0) 'batch hook runs an unsigned preview from another working directory'
            $env:WINDOWS_SIGNING_REQUIRED = 'true'
            & $wrapper $file 2>$null
            Assert-True ($LASTEXITCODE -ne 0) 'batch hook propagates a required-signing failure'
        } finally { Pop-Location }
    }
    $global:LASTEXITCODE = 0
    Write-Host "$passed Windows signing regression assertions passed."
} finally {
    foreach ($name in $previous.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name]) }
    Remove-Item -LiteralPath $fixture -Recurse -Force
}
