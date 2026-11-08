# Manifest/version tests are platform-neutral; Windows SDK packaging is tested in the Store workflow.
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/../msix.ps1"
$passed = 0
function Assert-True($Condition, [string]$Description) {
    if (-not $Condition) { throw "FAIL: $Description" }
    $script:passed++
    Write-Host "PASS: $Description"
}
function Assert-Throws([scriptblock]$Action) {
    $failed = $false
    try { & $Action | Out-Null } catch { $failed = $true }
    Assert-True $failed 'invalid version/payload is rejected'
}
$fixture = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
$null = New-Item -ItemType Directory -Path $fixture
try {
    Assert-True ((ConvertTo-NotoirStoreVersion '0.1.11') -eq '1.1.11.0') 'Cargo major zero maps to an accepted Store major'
    Assert-True ((ConvertTo-NotoirStoreVersion '1.0.0') -eq '2.0.0.0') 'future major releases cannot downgrade the Store version'
    Assert-True ((ConvertTo-NotoirStoreVersion '65534.65535.65535') -eq '65535.65535.65535.0') 'Store version bounds are supported'
    foreach ($bad in @('0.1.11-1', '0.1.11+build', '0.1', '01.2.3', '65535.0.0', '1.65536.0', '1.0.65536', '-1.0.0')) {
        Assert-Throws { ConvertTo-NotoirStoreVersion $bad }
    }
    $identity = Get-NotoirStoreIdentity
    Assert-True ($identity.Name -ceq 'LusFernandoSaquetimVendra.Notoir') 'reserved package name is exact'
    Assert-True ($identity.Publisher -ceq 'CN=EC133CA5-BEED-40F2-8541-21EEF52E4CF7') 'publisher GUID is exact'
    Assert-True ($identity.StoreId -ceq '9P1JPTK7G5PX') 'Store ID is preserved'
    $manifest = New-NotoirStoreManifest -Identity $identity -Version '0.1.11' -Languages @('en-US', 'pt-BR', 'zh-CN')
    $path = Join-Path $fixture 'AppxManifest.xml'
    Save-NotoirStoreManifest -Manifest $manifest -Path $path
    $parsed = [xml](Get-Content -LiteralPath $path -Raw -Encoding utf8)
    Assert-True ($parsed.Package.Identity.Name -ceq $identity.Name) 'generated identity name matches Partner Center'
    Assert-True ($parsed.Package.Identity.Publisher -ceq $identity.Publisher) 'generated publisher matches Partner Center'
    Assert-True ($parsed.Package.Identity.Version -ceq '1.1.11.0') 'revision is reserved as zero'
    Assert-True ($parsed.Package.Identity.ProcessorArchitecture -ceq 'x64') 'payload targets x64'
    Assert-True ($parsed.Package.Properties.PublisherDisplayName -ceq 'Luís Fernando Saquetim Vendrame') 'UTF-8 publisher display name survives serialization'
    Assert-True (@($parsed.Package.Resources.Resource).Count -eq 3) 'supported language resources are preserved'
    $ns = [Xml.XmlNamespaceManager]::new($parsed.NameTable)
    $ns.AddNamespace('m', $parsed.DocumentElement.NamespaceURI)
    $ns.AddNamespace('uap3', 'http://schemas.microsoft.com/appx/manifest/uap/windows10/3')
    $ns.AddNamespace('desktop', 'http://schemas.microsoft.com/appx/manifest/desktop/windows10')
    $ns.AddNamespace('rescap', 'http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities')
    Assert-True ($parsed.SelectSingleNode('//desktop:ExecutionAlias', $ns).Alias -ceq 'notoir.exe') 'package declares the command alias'
    Assert-True ($parsed.SelectSingleNode('//uap3:FileTypeAssociation', $ns).Parameters -ceq '"%1"') 'file activation quotes paths with spaces'
    Assert-True ($parsed.SelectSingleNode('//rescap:Capability', $ns).Name -ceq 'runFullTrust') 'classic Win32 capability is declared'
    $identity.PublisherDisplayName = 'A & B <Publisher>'
    Save-NotoirStoreManifest -Manifest (New-NotoirStoreManifest $identity '0.1.11') -Path $path
    $escaped = [xml](Get-Content -LiteralPath $path -Raw -Encoding utf8)
    Assert-True ($escaped.Package.Properties.PublisherDisplayName -ceq 'A & B <Publisher>') 'publisher strings are XML data rather than markup'

    $exe = Join-Path $fixture 'fixture.exe'
    $bytes = [byte[]]::new(256)
    $bytes[0] = 0x4d; $bytes[1] = 0x5a; $bytes[0x3c] = 0x80
    $bytes[0x80] = 0x50; $bytes[0x81] = 0x45; $bytes[0x84] = 0x64; $bytes[0x85] = 0x86
    [IO.File]::WriteAllBytes($exe, $bytes)
    Assert-NotoirStoreExecutable $exe
    Assert-True $true 'PE architecture validation accepts x64'
    $bytes[0x84] = 0x4c; $bytes[0x85] = 0x01
    [IO.File]::WriteAllBytes($exe, $bytes)
    Assert-Throws { Assert-NotoirStoreExecutable $exe }
    $bytes[0] = 0
    [IO.File]::WriteAllBytes($exe, $bytes)
    Assert-Throws { Assert-NotoirStoreExecutable $exe }

    if ($IsWindows) {
        # Exercise SDK schema/asset validation before compiling the large GUI.
        # This stand-in payload is never installed, uploaded or distributed.
        $payload = Join-Path $fixture 'sdk-payload'
        $null = New-Item -ItemType Directory -Path $payload
        Copy-Item -LiteralPath (Join-Path $PSHOME 'pwsh.exe') -Destination (Join-Path $payload 'notoir.exe')
        Save-NotoirStoreManifest -Manifest (New-NotoirStoreManifest (Get-NotoirStoreIdentity) '0.1.11') -Path (Join-Path $payload 'AppxManifest.xml')
        New-NotoirStoreLogos -Source (Join-Path $script:NotoirStoreRoot 'assets/app-icon/png/1024x1024.png') -Destination (Join-Path $payload 'Assets')
        $makeappx = Get-NotoirWindowsSdkTool -Name makeappx.exe
        & $makeappx pack /d $payload /p (Join-Path $fixture 'sdk-check.msix') /o
        Assert-True ($LASTEXITCODE -eq 0) 'Windows SDK validates the manifest, file association, alias and PNG assets'
    }
    $global:LASTEXITCODE = 0
    Write-Host "$passed MSIX regression assertions passed."
} finally { Remove-Item -LiteralPath $fixture -Recurse -Force }
