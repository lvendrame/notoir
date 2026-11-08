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
