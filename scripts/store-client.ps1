# Run only on a disposable CI runner. The secret comes from GitHub's environment.
param(
    [ValidateSet('VerifyAccess', 'Publish')][string]$Mode = 'VerifyAccess',
    [string]$PackageDirectory = "$PSScriptRoot/../target/store-msix"
)
$ErrorActionPreference = 'Stop'
foreach ($name in @('AZURE_AD_TENANT_ID', 'AZURE_AD_APPLICATION_CLIENT_ID', 'AZURE_AD_APPLICATION_SECRET', 'SELLER_ID')) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        throw "Missing GitHub Actions secret: $name"
    }
}
$identity = Get-Content -Raw "$PSScriptRoot/../packaging/windows-store/identity.json" | ConvertFrom-Json
$package = $null
$configured = $false
if ($Mode -eq 'Publish') {
    $out = $PackageDirectory
    $info = Get-Content -Raw (Join-Path $out 'submission-info.json') | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_SHA) -or
        $info.StoreId -ne $identity.StoreId -or $info.SourceCommit -ne $env:GITHUB_SHA) {
        throw 'Store artifact does not belong to this product and candidate commit.'
    }
    if ($info.Package -ne [IO.Path]::GetFileName($info.Package) -or $info.Package -notlike '*.msix') {
        throw 'Invalid MSIX artifact filename.'
    }
    $package = (Resolve-Path (Join-Path $out $info.Package)).Path
    if ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash -ine $info.Sha256) {
        throw 'Store artifact checksum does not match submission-info.json.'
    }
}
try {
    & msstore reconfigure --tenantId $env:AZURE_AD_TENANT_ID --sellerId $env:SELLER_ID `
        --clientId $env:AZURE_AD_APPLICATION_CLIENT_ID --clientSecret $env:AZURE_AD_APPLICATION_SECRET
    if ($LASTEXITCODE -ne 0) { throw 'Microsoft Store CLI configuration failed.' }
    $configured = $true
    # This is read-only: it verifies actual product access, not just secret presence.
    & msstore apps get $identity.StoreId
    if ($LASTEXITCODE -ne 0) { throw 'Microsoft Store product access failed; check the tenant association and Manager role.' }
    if ($Mode -eq 'Publish') {
        # The official package-update command carries forward the existing listing.
        # Do not supply metadata JSON or override the existing price.
        & msstore publish $package -id $identity.StoreId
        if ($LASTEXITCODE -ne 0) { throw 'Microsoft Store submission failed. Inspect its state before retrying.' }
        "Submitted $($info.PackageVersion) to Microsoft Store for certification." |
            Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
    }
} finally {
    & cmdkey.exe "/delete:MicrosoftStoreCli:user=$env:AZURE_AD_APPLICATION_CLIENT_ID" | Out-Null
    $credentialRemoved = $LASTEXITCODE -eq 0
    $cliProfilePath = Join-Path $env:LOCALAPPDATA 'Microsoft/MSStore.CLI'
    if (Test-Path -LiteralPath $cliProfilePath) { Remove-Item -LiteralPath $cliProfilePath -Recurse -Force }
    if ($configured -and -not $credentialRemoved) { throw 'Could not remove the saved Microsoft Store CLI credential.' }
    $global:LASTEXITCODE = 0
}
