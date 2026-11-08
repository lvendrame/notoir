$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/windows-sdk.ps1"
$script:NotoirStoreRoot = (Resolve-Path "$PSScriptRoot/..").Path

function Get-NotoirStoreIdentity {
    param([string]$Path = (Join-Path $script:NotoirStoreRoot 'packaging/windows-store/identity.json'))
    $identity = Get-Content -LiteralPath $Path -Raw -Encoding utf8 | ConvertFrom-Json
    foreach ($field in @('Name', 'Publisher', 'PublisherDisplayName', 'PackageFamilyName', 'StoreId', 'ApplicationId')) {
        if ([string]::IsNullOrWhiteSpace($identity.$field)) { throw "Missing Store identity field: $field" }
    }
    if ($identity.Name -notmatch '^[A-Za-z0-9][A-Za-z0-9.-]{2,49}$' -or
        -not $identity.PackageFamilyName.StartsWith("$($identity.Name)_", [StringComparison]::Ordinal)) {
        throw 'Invalid Store identity name/package family.'
    }
    return $identity
}

function ConvertTo-NotoirStoreVersion {
    param([Parameter(Mandatory)][string]$Version)
    if ($Version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
        throw 'Store packaging requires a stable Cargo major.minor.patch version.'
    }
    try { $parts = @($Matches[1], $Matches[2], $Matches[3]) | ForEach-Object { [long]$_ } }
    catch { throw 'Store version components exceed their supported range.' }
    # Store major cannot be zero, and revision is reserved for Microsoft.
    # Offset every Cargo major, including 1.x and later, to remain monotonic.
    if ($parts[0] -gt 65534 -or $parts[1] -gt 65535 -or $parts[2] -gt 65535) {
        throw 'Store version components exceed their supported range.'
    }
    return "$($parts[0] + 1).$($parts[1]).$($parts[2]).0"
}

function New-NotoirStoreManifest {
    param(
        [Parameter(Mandatory)]$Identity,
        [Parameter(Mandatory)][string]$Version,
        [string]$Template = (Join-Path $script:NotoirStoreRoot 'packaging/windows-store/AppxManifest.xml'),
        [string[]]$Languages = @('en-US')
    )
    $manifest = [xml](Get-Content -LiteralPath $Template -Raw -Encoding utf8)
    $manifest.Package.Identity.SetAttribute('Name', $Identity.Name)
    $manifest.Package.Identity.SetAttribute('Publisher', $Identity.Publisher)
    $manifest.Package.Identity.SetAttribute('Version', (ConvertTo-NotoirStoreVersion $Version))
    $manifest.Package.Properties.PublisherDisplayName = [string]$Identity.PublisherDisplayName
    $manifest.Package.Applications.Application.SetAttribute('Id', $Identity.ApplicationId)
    $resources = $manifest.Package.Resources
    $resources.RemoveAll()
    foreach ($language in $Languages) {
        $resource = $manifest.CreateElement('Resource', $manifest.DocumentElement.NamespaceURI)
        $resource.SetAttribute('Language', $language)
        $null = $resources.AppendChild($resource)
    }
    return ,$manifest
}

function Save-NotoirStoreManifest {
    param([Parameter(Mandatory)][xml]$Manifest, [Parameter(Mandatory)][string]$Path)
    $settings = [Xml.XmlWriterSettings]::new()
    $settings.Encoding = [Text.UTF8Encoding]::new($false)
    $settings.Indent = $true
    $writer = [Xml.XmlWriter]::Create($Path, $settings)
    try { $Manifest.Save($writer) } finally { $writer.Dispose() }
}

function Assert-NotoirStoreExecutable {
    param([Parameter(Mandatory)][string]$Path)
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        if ($reader.BaseStream.Length -lt 128 -or $reader.ReadUInt16() -ne 0x5a4d) { throw 'Store payload is not a PE executable.' }
        $reader.BaseStream.Position = 0x3c
        $offset = $reader.ReadUInt32()
        if ($offset -gt $reader.BaseStream.Length - 6) { throw 'Invalid PE header offset.' }
        $reader.BaseStream.Position = $offset
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) { throw 'Store payload must be an x64 Windows executable.' }
    } finally { $reader.Dispose() }
}

function New-NotoirStoreLogos {
    param([Parameter(Mandatory)][string]$Source, [Parameter(Mandatory)][string]$Destination)
    Add-Type -AssemblyName System.Drawing
    $null = New-Item -ItemType Directory -Path $Destination -Force
    $sourceImage = [Drawing.Image]::FromFile($Source)
    try {
        foreach ($logo in @{ StoreLogo = 50; Square44x44Logo = 44; Square150x150Logo = 150 }.GetEnumerator()) {
            foreach ($scale in @(1, 2, 4)) {
                $size = $logo.Value * $scale
                $bitmap = [Drawing.Bitmap]::new($size, $size)
                $graphics = [Drawing.Graphics]::FromImage($bitmap)
                try {
                    $graphics.Clear([Drawing.Color]::Transparent)
                    $graphics.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceCopy
                    $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                    $graphics.DrawImage($sourceImage, 0, 0, $size, $size)
                    $suffix = if ($scale -eq 1) { '' } else { ".scale-$($scale * 100)" }
                    $bitmap.Save((Join-Path $Destination "$($logo.Key)$suffix.png"), [Drawing.Imaging.ImageFormat]::Png)
                } finally { $graphics.Dispose(); $bitmap.Dispose() }
            }
        }
    } finally { $sourceImage.Dispose() }
}

function Assert-NotoirStoreRuntime {
    param([Parameter(Mandatory)][string]$Executable, [Parameter(Mandatory)][string]$Report)
    $tool = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
    if ($tool) { $dumpbin = $tool.Source }
    else {
        $vswhere = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'Microsoft Visual Studio/Installer/vswhere.exe'
        $tools = @(& $vswhere -latest -products '*' -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe')
        if ($LASTEXITCODE -ne 0 -or $tools.Count -eq 0) { throw 'Visual Studio C++ dumpbin.exe is required to verify Store runtime dependencies.' }
        $dumpbin = $tools[0]
    }
    $dependencies = & $dumpbin /dependents $Executable
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect Store executable imports.' }
    $dependencies | Set-Content -LiteralPath $Report -Encoding utf8
    if (($dependencies -join "`n") -match '(?im)^\s+(?:vcruntime[^\s]*|msvcp[^\s]*|ucrtbase|api-ms-win-crt-[^\s]*)\.dll\s*$') {
        throw 'Store payload requires a dynamic VC runtime. Build with -C target-feature=+crt-static.'
    }
}
