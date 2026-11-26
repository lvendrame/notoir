function Get-NotoirWindowsLinker {
    param([Parameter(Mandatory)][string]$Expected)
    # An exact executable name can return every PATH match in PowerShell.
    $resolved = (Get-Command link.exe -CommandType Application -ErrorAction Stop |
        Select-Object -First 1).Source
    if (-not $resolved -or $resolved -ine $Expected) {
        throw "Wrong Windows linker: PATH resolves '$resolved'; expected '$Expected'."
    }
    return $resolved
}

function Assert-NotoirWindowsMediaLibraries {
    param([Parameter(Mandatory)][string]$Prefix)
    $missing = @('avcodec', 'avformat', 'avutil', 'swresample', 'swscale') | Where-Object {
        $path = Join-Path $Prefix "lib/$_.lib"
        -not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-Item -LiteralPath $path).Length -eq 0
    }
    if ($missing) {
        throw "Missing Windows FFmpeg import libraries in '$Prefix/lib': $($missing -join ', ')"
    }
}

function Get-NotoirWindowsSdkTool {
    param([Parameter(Mandatory)][ValidateSet('signtool.exe', 'makeappx.exe')][string]$Name)
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $root = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'Windows Kits/10/bin'
    $tools = @(Get-ChildItem -Path "$root/*/x64/$Name" -ErrorAction SilentlyContinue |
        Where-Object { $_.Directory.Parent.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Directory.Parent.Name } -Descending)
    if ($tools.Count -eq 0) { throw "Install the Windows SDK containing x64 $Name." }
    return $tools[0].FullName
}
