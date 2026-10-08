# Regression run: launch each IPA on a device for N seconds and report whether it survived.
#   .\regress.ps1                       # the known-good list (Working games in GAMES_STATUS.md)
#   .\regress.ps1 -All                  # every IPA in the device's touchHLE_apps folder
#   .\regress.ps1 -Apps "Pocket God 1.39.ipa","Sonic 1 1.2.6.ipa" -Wait 40
# Writes REGRESSION.md next to this folder and pushes compat.json (per-IPA outcome) to the
# device so the launcher can show a status badge on each tile. Screenshots stay in -ShotDir
# (kept out of the project) and are only taken while the game window has focus.
param(
    [string[]]$Apps,
    [switch]$All,
    [int]$Wait = 25,
    [string]$Serial = "R52Y8066STA",
    [string]$Server = "5038",
    [string]$ShotDir = "$env:TEMP\playcover_regress_shots",
    [switch]$NoPush
)
$ErrorActionPreference = "Continue"
$a = @("-P", $Server, "-s", $Serial)
$pkg = "org.touchhle.android"
$files = "/sdcard/Android/data/$pkg/files"
$root = Split-Path -Parent $PSScriptRoot
$default = @("Sonic 1 1.2.6.ipa", "Sonic 2 1.2.2.ipa", "Pocket God 1.39.ipa", "Rock Band 1.1.38.ipa", "FlappyBird_1.2.ipa")
if ($All) { $Apps = (adb @a shell "ls $files/touchHLE_apps") | Where-Object { $_ -like "*.ipa" } }
if (-not $Apps) { $Apps = $default }
New-Item -ItemType Directory -Force $ShotDir | Out-Null
$results = [ordered]@{}
$rows = @()
foreach ($app in $Apps) {
    adb @a shell am force-stop $pkg | Out-Null
    adb @a shell "rm -f $files/touchHLE_log.txt" | Out-Null
    adb @a shell "am start -n $pkg/$pkg.MainActivity --es app_path '$files/touchHLE_apps/$app' --ez compat true" | Out-Null
    Start-Sleep $Wait
    $focus = ((adb @a shell "dumpsys window | grep mCurrentFocus") -join " ")
    $alive = $focus -match [regex]::Escape("$pkg/")
    $logFile = Join-Path $ShotDir "log.txt"
    adb @a pull "$files/touchHLE_log.txt" $logFile 2>&1 | Out-Null
    $log = if (Test-Path $logFile) { Get-Content $logFile } else { @() }
    $panic = $log | Select-String -Pattern "^Panic at" | Select-Object -First 1
    $guest = $log | Select-String -Pattern "Guest (read|write) error" | Select-Object -First 1
    $status = if ($panic -or $guest) { "crashes" } elseif ($alive) { "runs" } else { "exited" }
    $note = if ($panic) { $panic.Line } elseif ($guest) { $guest.Line } elseif (-not $alive) { "left foreground / process ended" } else { "" }
    $note = $note.Substring(0, [Math]::Min(160, $note.Length))
    # Functions the app called that touchHLE does not implement (each returns 0): the likely next fixes.
    $missing = @($log | Select-String -Pattern "Ignoring call to unimplemented function (\S+)" | ForEach-Object { $_.Matches[0].Groups[1].Value } | Select-Object -Unique | Select-Object -First 6)
    if ($missing.Count -gt 0) { $note = ($note + " [unimplemented calls: " + ($missing -join ", ") + "]").Trim() }
    if ($alive) {
        $safe = ($app -replace '[^\w\.\-]', '_')
        adb @a shell "screencap -p /data/local/tmp/r.png" | Out-Null
        adb @a pull /data/local/tmp/r.png (Join-Path $ShotDir "$safe.png") 2>&1 | Out-Null
        adb @a shell "rm /data/local/tmp/r.png" | Out-Null
    }
    $results[$app] = @{ status = $status; note = $note; when = (Get-Date -Format "yyyy-MM-dd") }
    $rows += "| $app | $status | $($log.Count) | " + ($note -replace '\|', '/') + " |"
    "{0,-48} {1}" -f $app, $status
}
adb @a shell am force-stop $pkg | Out-Null

# Merge into the existing table so a run of a few games does not wipe the rest.
$mdPath = Join-Path $root "REGRESSION.md"
$table = [ordered]@{}
if (Test-Path $mdPath) {
    foreach ($line in Get-Content $mdPath) {
        if ($line -match '^\| (.+?\.ipa) \|') { $table[$Matches[1]] = $line }
    }
}
foreach ($row in $rows) {
    if ($row -match '^\| (.+?\.ipa) \|') { $table[$Matches[1]] = $row }
}
$sorted = $table.Keys | Sort-Object { $_.ToLower() } | ForEach-Object { $table[$_] }
$stamp = Get-Date -Format 'yyyy-MM-dd HH:mm'
$md = @('# Regression run', '', "Device $Serial. Each row is updated by tools/regress.ps1 when that game is run (last write $stamp, $Wait s per game). 'runs' = still foreground with no panic, not proof it plays correctly.", '', '| IPA | Result | Log lines | First problem |', '|---|---|---|---|') + $sorted
Set-Content -Path $mdPath -Value $md -Encoding utf8

if (-not $NoPush) {
    # merge with any existing compat.json so a partial run keeps other entries
    $existing = @{}
    $tmp = Join-Path $ShotDir "compat.json"
    adb @a pull "$files/compat.json" $tmp 2>&1 | Out-Null
    if (Test-Path $tmp) { try { (Get-Content $tmp -Raw | ConvertFrom-Json).PSObject.Properties | ForEach-Object { $existing[$_.Name] = $_.Value } } catch {} }
    foreach ($k in $results.Keys) { $existing[$k] = $results[$k] }
    $existing | ConvertTo-Json -Depth 4 | Set-Content -Path $tmp -Encoding utf8
    adb @a push $tmp "$files/compat.json" 2>&1 | Out-Null
    adb @a shell "chmod 666 $files/compat.json" | Out-Null
}
"wrote REGRESSION.md"
