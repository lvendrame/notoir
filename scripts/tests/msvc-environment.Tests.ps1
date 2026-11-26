$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/../windows-sdk.ps1"

$microsoft = 'C:\Program Files\Microsoft Visual Studio\VC\bin\Hostx64\x64\link.exe'
$unix = 'C:\Program Files\Git\usr\bin\link.exe'
function Get-Command {
    param($Name, $CommandType, $ErrorAction)
    if ($Name -ne 'link.exe') { throw "Unexpected command lookup: $Name" }
    return $script:linkers
}

try {
    $script:linkers = @([pscustomobject]@{ Source = $microsoft }, [pscustomobject]@{ Source = $unix })
    $actual = Get-NotoirWindowsLinker -Expected $microsoft
    if ($actual -cne $microsoft) { throw "Did not select the first PATH match: $actual" }
    Write-Host 'PASS: Microsoft linker first, Unix linker later'

    $script:linkers = @([pscustomobject]@{ Source = $unix }, [pscustomobject]@{ Source = $microsoft })
    $rejected = $false
    try { Get-NotoirWindowsLinker -Expected $microsoft | Out-Null }
    catch {
        if ($_.Exception.Message -notlike 'Wrong Windows linker:*') { throw }
        $rejected = $true
    }
    if (-not $rejected) { throw 'Accepted the Unix linker before the Microsoft linker.' }
    Write-Host 'PASS: Unix linker first is rejected'

    $prefix = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
    try {
        $null = New-Item -ItemType Directory -Path (Join-Path $prefix 'lib') -Force
        foreach ($name in @('avcodec', 'avformat', 'avutil', 'swresample', 'swscale')) {
            [IO.File]::WriteAllText((Join-Path $prefix "lib/$name.lib"), 'import library fixture')
        }
        Assert-NotoirWindowsMediaLibraries -Prefix $prefix
        Write-Host 'PASS: all five canonical import-library paths are accepted'

        $library = Join-Path $prefix 'lib/swscale.lib'
        foreach ($empty in @($false, $true)) {
            if ($empty) { [IO.File]::WriteAllBytes($library, [byte[]]@()) }
            else { Remove-Item -LiteralPath $library }
            $rejected = $false
            try { Assert-NotoirWindowsMediaLibraries -Prefix $prefix }
            catch {
                if ($_.Exception.Message -notlike 'Missing Windows FFmpeg import libraries*swscale*') { throw }
                $rejected = $true
            }
            if (-not $rejected) { throw "Accepted an absent/empty import library (empty=$empty)." }
        }
        Write-Host 'PASS: missing and empty import libraries are rejected before Cargo'
    } finally {
        if (Test-Path -LiteralPath $prefix) { Remove-Item -LiteralPath $prefix -Recurse -Force }
    }
} finally {
    Remove-Item Function:Get-Command
}
