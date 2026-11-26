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
} finally {
    Remove-Item Function:Get-Command
}
