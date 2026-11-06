# Shared by the release preflight, cargo-packager signing hook and offline tests.
$ErrorActionPreference = 'Stop'
$script:ArtifactSigningModuleVersion = '0.1.20'

function Assert-NotoirWindowsSigningConfiguration {
    param(
        [switch]$Required,
        [Nullable[bool]]$CertificateAvailable,
        [Nullable[bool]]$PasswordAvailable
    )
    $provider = $env:WINDOWS_SIGNING_PROVIDER
    if ([string]::IsNullOrWhiteSpace($provider)) { $provider = 'none' }
    if ($provider -notin @('none', 'azure', 'pfx')) {
        throw 'WINDOWS_SIGNING_PROVIDER must be azure, pfx or none.'
    }
    if ($provider -eq 'none') {
        if ($Required) {
            throw 'Windows publication requires Authenticode signing. Configure WINDOWS_SIGNING_PROVIDER; see docs/windows-signing.md.'
        }
        return $provider
    }
    if ([string]::IsNullOrWhiteSpace($env:WINDOWS_EXPECTED_PUBLISHER)) {
        throw 'WINDOWS_EXPECTED_PUBLISHER must match the verified certificate publisher.'
    }
    if ($provider -eq 'azure') {
        foreach ($name in @('AZURE_CLIENT_ID', 'AZURE_TENANT_ID', 'AZURE_SUBSCRIPTION_ID',
                            'AZURE_SIGNING_ENDPOINT', 'AZURE_SIGNING_ACCOUNT', 'AZURE_SIGNING_PROFILE')) {
            if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
                throw "Missing signing configuration: $name"
            }
        }
        if ($env:AZURE_SIGNING_ENDPOINT -notmatch '^https://[a-z0-9-]+\.codesigning\.azure\.net/?$') {
            throw 'AZURE_SIGNING_ENDPOINT must be an HTTPS regional codesigning.azure.net endpoint.'
        }
    } else {
        if ($null -eq $CertificateAvailable) {
            $CertificateAvailable = -not [string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE_PATH) -and
                (Test-Path -LiteralPath $env:WINDOWS_CERTIFICATE_PATH -PathType Leaf)
        }
        if ($null -eq $PasswordAvailable) {
            $PasswordAvailable = -not [string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE_PASSWORD)
        }
        if (-not $CertificateAvailable -or -not $PasswordAvailable) {
            throw 'PFX signing requires WINDOWS_CERTIFICATE and WINDOWS_CERTIFICATE_PASSWORD (or a local WINDOWS_CERTIFICATE_PATH).'
        }
    }
    return $provider
}

function Get-NotoirSignTool {
    $command = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $root = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'Windows Kits/10/bin'
    $tools = @(Get-ChildItem -Path "$root/*/x64/signtool.exe" -ErrorAction SilentlyContinue |
        Sort-Object { [version]$_.Directory.Parent.Name } -Descending)
    if ($tools.Count -eq 0) { throw 'Install the Windows SDK containing x64 signtool.exe.' }
    return $tools[0].FullName
}

function Test-NotoirWindowsSignature {
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Missing signing output: $Path" }
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ([string]$signature.Status -ne 'Valid' -or $null -eq $signature.SignerCertificate) {
        throw "Invalid Authenticode signature: $Path ($($signature.Status))"
    }
    if ($null -eq $signature.TimeStamperCertificate) {
        throw "Missing Authenticode timestamp: $Path"
    }
    $publisher = $signature.SignerCertificate.GetNameInfo(
        [System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false)
    if ([string]::IsNullOrWhiteSpace($env:WINDOWS_EXPECTED_PUBLISHER) -or
        $publisher -cne $env:WINDOWS_EXPECTED_PUBLISHER) {
        throw "Unexpected Authenticode publisher '$publisher' for $Path; expected '$env:WINDOWS_EXPECTED_PUBLISHER'."
    }
    Write-Host "Verified Authenticode publisher '$publisher': $Path"
}

function Write-NotoirWindowsSigningReceipt {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][bool]$Signed)
    if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_RECEIPTS)) { return }
    $file = (Resolve-Path -LiteralPath $Path).Path
    $name = [IO.Path]::GetFileName($file)
    $kind = if ($name -eq 'notoir.exe') { 'app' }
        elseif ($name.EndsWith('.msi', [StringComparison]::OrdinalIgnoreCase)) { 'msi' }
        elseif ($name.EndsWith('-setup.exe', [StringComparison]::OrdinalIgnoreCase)) { 'nsis' }
        elseif ($env:WINDOWS_SIGNING_PHASE -eq 'nsis') { 'uninstaller' }
        else { throw "Unexpected file in signing hook: $Path" }
    $receipt = @{
        Kind = $kind
        Path = $file
        Sha256 = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash
        Signed = $Signed
        Publisher = $(if ($Signed) { $env:WINDOWS_EXPECTED_PUBLISHER } else { $null })
    }
    $receipt | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $env:WINDOWS_SIGNING_RECEIPTS "$kind.json") -Encoding utf8
}

function Test-NotoirWindowsSigningReceipts {
    $provider = Assert-NotoirWindowsSigningConfiguration -Required:($env:WINDOWS_SIGNING_REQUIRED -eq 'true')
    if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_RECEIPTS)) { throw 'Missing Windows signing receipt directory.' }
    $signed = $provider -ne 'none'
    foreach ($kind in @('app', 'msi', 'nsis', 'uninstaller')) {
        $path = Join-Path $env:WINDOWS_SIGNING_RECEIPTS "$kind.json"
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing $kind signing receipt." }
        $receipt = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
        if ($receipt.Kind -ne $kind -or $receipt.Signed -isnot [bool] -or $receipt.Signed -ne $signed -or
            ($signed -and $receipt.Publisher -cne $env:WINDOWS_EXPECTED_PUBLISHER)) {
            throw "$kind receipt does not match the configured signing identity."
        }
        # NSIS deletes its temporary uninstaller after embedding it. Its
        # receipt exists only if that file passed verification in the hook.
        if ($kind -ne 'uninstaller') {
            if (-not (Test-Path -LiteralPath $receipt.Path -PathType Leaf) -or
                (Get-FileHash -LiteralPath $receipt.Path -Algorithm SHA256).Hash -ne $receipt.Sha256) {
                throw "Changed signing output: $($receipt.Path)"
            }
            if ($signed) { Test-NotoirWindowsSignature -Path $receipt.Path }
        }
    }
    Write-Host "All packaging hooks completed: app, MSI, NSIS installer and uninstaller (signed=$signed)."
}

function Invoke-NotoirWindowsSign {
    param([Parameter(Mandatory)][string]$Path)
    $provider = Assert-NotoirWindowsSigningConfiguration -Required
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Missing file to sign: $Path" }
    $file = (Resolve-Path -LiteralPath $Path).Path
    if ($provider -eq 'azure') {
        Import-Module ArtifactSigning -RequiredVersion $script:ArtifactSigningModuleVersion -ErrorAction Stop
        # Use only the Azure CLI session established by azure/login with OIDC.
        # In particular, never fall back to an interactive browser on a runner.
        $parameters = @{
            Endpoint = $env:AZURE_SIGNING_ENDPOINT
            CodeSigningAccountName = $env:AZURE_SIGNING_ACCOUNT
            CertificateProfileName = $env:AZURE_SIGNING_PROFILE
            Files = $file
            FileDigest = 'SHA256'
            TimestampRfc3161 = 'http://timestamp.acs.microsoft.com'
            TimestampDigest = 'SHA256'
            ExcludeEnvironmentCredential = $true
            ExcludeWorkloadIdentityCredential = $true
            ExcludeManagedIdentityCredential = $true
            ExcludeSharedTokenCacheCredential = $true
            ExcludeVisualStudioCredential = $true
            ExcludeVisualStudioCodeCredential = $true
            ExcludeAzureCliCredential = $false
            ExcludeAzurePowerShellCredential = $true
            ExcludeAzureDeveloperCliCredential = $true
            ExcludeInteractiveBrowserCredential = $true
        }
        Invoke-ArtifactSigning @parameters
    } else {
        $tool = Get-NotoirSignTool
        # Invoke directly from PowerShell: Git Bash must not rewrite /f, /p etc.
        & $tool sign /q /a /f $env:WINDOWS_CERTIFICATE_PATH /p $env:WINDOWS_CERTIFICATE_PASSWORD `
            /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $file
        if ($LASTEXITCODE -ne 0) { throw "SignTool failed for $Path (exit $LASTEXITCODE)." }
    }
    Test-NotoirWindowsSignature -Path $file
    Write-NotoirWindowsSigningReceipt -Path $file -Signed $true
}
