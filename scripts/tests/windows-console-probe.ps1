# Fixture for Rust tests: report the actual console handle and exercise pipes.
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class ConsoleProbe { [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow(); }'
[Console]::WriteLine([ConsoleProbe]::GetConsoleWindow().ToInt64())
[Console]::WriteLine([Console]::ReadLine())
[Console]::Error.WriteLine('helper stderr')
exit 7
