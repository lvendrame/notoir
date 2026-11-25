param([Parameter(Mandatory)][string]$Directory)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
try {
    $request = Get-Content -LiteralPath (Join-Path $Directory 'request.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $own = 'Notoir.File'
    $aumid = $null
    if ($request.store) {
        $package = Get-AppxPackage | Where-Object { $_.InstallLocation -and $request.executable.StartsWith($_.InstallLocation + '\', [StringComparison]::OrdinalIgnoreCase) } | Select-Object -First 1
        if (!$package) { throw 'Cannot locate the installed Notoir package.' }
        $manifest = Get-AppxPackageManifest -Package $package.PackageFullName
        $application = @($manifest.Package.Applications.Application) | Where-Object { $_.Executable -and [IO.Path]::GetFileName($_.Executable) -eq 'notoir.exe' } | Select-Object -First 1
        if (!$application) { throw 'Cannot locate the packaged application identity.' }
        $aumid = $package.PackageFamilyName + '!' + $application.Id
    }
    switch ($request.action) {
        'inspect' {
            Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class NotoirAssociations {
 [DllImport("Shlwapi.dll", CharSet=CharSet.Unicode)]
 static extern uint AssocQueryString(uint flags,uint str,string assoc,string extra,StringBuilder output,ref uint size);
 public static string Query(string extension, uint kind) {
  uint size=0; AssocQueryString(0,kind,extension,null,null,ref size);
  if(size==0) return null;
  var value=new StringBuilder((int)size);
  return AssocQueryString(0,kind,extension,null,value,ref size)==0 ? value.ToString() : null;
 }
}
'@
            $rows = @($request.extensions | ForEach-Object {
                $extension = $_
                $handler = [NotoirAssociations]::Query('.' + $extension, 20)
                $isNotoir = $handler -eq $own
                if (!$request.store -and !$isNotoir) {
                    $executable = [NotoirAssociations]::Query('.' + $extension, 2)
                    $isNotoir = $executable -and [string]::Equals($executable, $request.executable, [StringComparison]::OrdinalIgnoreCase)
                }
                if (!$isNotoir -and $handler) {
                    $key = [Microsoft.Win32.Registry]::ClassesRoot.OpenSubKey($handler + '\Application')
                    if ($key) {
                        $registered = $key.GetValue('AppUserModelID')
                        $isNotoir = $aumid -and $registered -eq $aumid
                        $key.Dispose()
                    }
                }
                @{extension=$extension; content_type=('.' + $extension); handler=$handler; is_notoir=[bool]$isNotoir}
            })
            ConvertTo-Json -InputObject $rows -Compress
        }
        'register' {
            if ($request.store) { break }
            $root = 'HKCU:\Software\Classes\Notoir.File'
            New-Item -Path ($root + '\shell\open\command') -Force | Out-Null
            Set-Item -LiteralPath ($root + '\shell\open\command') -Value ('"' + $request.executable + '" -- "%1"')
            New-Item -Path ($root + '\DefaultIcon') -Force | Out-Null
            Set-Item -LiteralPath ($root + '\DefaultIcon') -Value ('"' + $request.executable + '",0')
            $cap = 'HKCU:\Software\Notoir\Capabilities'
            New-Item -Path ($cap + '\FileAssociations') -Force | Out-Null
            New-ItemProperty -Path $cap -Name ApplicationName -Value Notoir -Force | Out-Null
            New-ItemProperty -Path $cap -Name ApplicationDescription -Value Notoir -Force | Out-Null
            foreach ($extension in $request.extensions) {
                New-ItemProperty -Path ($cap + '\FileAssociations') -Name ('.' + $extension) -Value $own -Force | Out-Null
                $openWith = 'HKCU:\Software\Classes\.' + $extension + '\OpenWithProgids'
                New-Item -Path $openWith -Force | Out-Null
                New-ItemProperty -Path $openWith -Name $own -Value '' -Force | Out-Null
            }
            New-Item -Path 'HKCU:\Software\RegisteredApplications' -Force | Out-Null
            New-ItemProperty -Path 'HKCU:\Software\RegisteredApplications' -Name Notoir -Value 'Software\Notoir\Capabilities' -Force | Out-Null
        }
        'change' {
            # Windows requires the user to choose. Never alter UserChoice/hash.
        }
        'confirm' {
            if ($aumid) { $uri = 'ms-settings:defaultapps?registeredAUMID=' + [Uri]::EscapeDataString($aumid) }
            else { $uri = 'ms-settings:defaultapps?registeredAppUser=Notoir' }
            Start-Process $uri
        }
        default { throw 'Unknown association action.' }
    }
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
