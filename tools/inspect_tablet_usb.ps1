Get-CimInstance Win32_PnPEntity |
    Where-Object { $_.PNPDeviceID -like 'USB\VID_04E8*' -or $_.Name -like '*ADB*' -or $_.Name -like '*Samsung*' } |
    Select-Object Name, PNPDeviceID, Status, ConfigManagerErrorCode | ConvertTo-Json
