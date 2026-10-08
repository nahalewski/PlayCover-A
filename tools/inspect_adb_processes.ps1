Get-CimInstance Win32_Process | Where-Object Name -eq 'adb.exe' |
    Select-Object ProcessId, ParentProcessId, CommandLine | ConvertTo-Json
