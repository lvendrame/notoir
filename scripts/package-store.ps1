# Generates an unsigned MSIX for Partner Center. Microsoft signs it after certification.
param()
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/msix.ps1"
if (-not $IsWindows) { throw 'Build the Store package on Windows with the Windows SDK installed.' }
$root = (Resolve-Path "$PSScriptRoot/..").Path
Push-Location $root
try {
    $identity = Get-NotoirStoreIdentity
    $packageId = & cargo pkgid -p notoir
    if ($LASTEXITCODE -ne 0) { throw 'Could not read the application version.' }
    $version = ($packageId -split '[#@]')[-1]
    $storeVersion = ConvertTo-NotoirStoreVersion $version
    $previousRustFlags = $env:RUSTFLAGS
    try {
        $env:RUSTFLAGS = "$previousRustFlags -C target-feature=+crt-static".Trim()
        & cargo build --locked --profile dist -p notoir --features microsoft-store
        if ($LASTEXITCODE -ne 0) { throw 'Store executable build failed.' }
    } finally { $env:RUSTFLAGS = $previousRustFlags }
    $exe = Join-Path $root 'target/dist/notoir.exe'
    Assert-NotoirStoreExecutable $exe
    # Only generated build output is cleaned. The direct-download packages remain separate.
    $out = Join-Path $root 'target/store-msix'
    if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Recurse -Force }
    $payload = Join-Path $out 'payload'
    $null = New-Item -ItemType Directory -Path $payload -Force
    Copy-Item -LiteralPath $exe -Destination (Join-Path $payload 'notoir.exe')
    Assert-NotoirStoreRuntime -Executable $exe -Report (Join-Path $out 'runtime-dependencies.txt')
    $languages = @(Get-ChildItem -LiteralPath 'assets/locales' -Directory | ForEach-Object {
        if ($_.Name -eq 'en') { 'en-US' } else { $_.Name }
    })
    $manifest = New-NotoirStoreManifest -Identity $identity -Version $version -Languages $languages
    Save-NotoirStoreManifest -Manifest $manifest -Path (Join-Path $payload 'AppxManifest.xml')
    New-NotoirStoreLogos -Source (Join-Path $root 'assets/app-icon/png/1024x1024.png') -Destination (Join-Path $payload 'Assets')
    $name = "notoir_${storeVersion}_x64_store.msix"
    $package = Join-Path $out $name
    $makeappx = Get-NotoirWindowsSdkTool -Name makeappx.exe
    # Default SDK semantic validation stays enabled; no /nv escape hatch.
    & $makeappx pack /d $payload /p $package /o
    if ($LASTEXITCODE -ne 0) { throw 'MakeAppx validation/packaging failed.' }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($package)
    try {
        if ($zip.GetEntry('AppxSignature.p7x')) { throw 'The Store submission must remain unsigned for Microsoft to sign.' }
        $entry = $zip.GetEntry('AppxManifest.xml')
        if (-not $entry) { throw 'MSIX is missing AppxManifest.xml.' }
        $reader = [IO.StreamReader]::new($entry.Open())
        try { $packed = [xml]$reader.ReadToEnd() } finally { $reader.Dispose() }
        if ($packed.Package.Identity.Name -cne $identity.Name -or
            $packed.Package.Identity.Publisher -cne $identity.Publisher -or
            $packed.Package.Identity.Version -ne $storeVersion -or
            $packed.Package.Properties.PublisherDisplayName -cne $identity.PublisherDisplayName) {
            throw 'Packed MSIX identity does not match the Partner Center reservation.'
        }
    } finally { $zip.Dispose() }
    $hash = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $name" | Set-Content -LiteralPath (Join-Path $out 'SHA256SUMS.txt') -Encoding utf8
    @{
        StoreId = $identity.StoreId; PackageFamilyName = $identity.PackageFamilyName
        Publisher = $identity.Publisher; PublisherDisplayName = $identity.PublisherDisplayName
        SourceVersion = $version; PackageVersion = $storeVersion; SourceCommit = $env:GITHUB_SHA
        Package = $name; Sha256 = $hash; Signing = 'Microsoft Store signs after certification'
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $out 'submission-info.json') -Encoding utf8
    Write-Host "Partner Center package: $package"
} finally { Pop-Location }
