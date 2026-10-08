# AGENT VERSION of cycle.ps1: builds from touchHLE-src-agent (WSL ~/touchHLE-agent), installs on the Pixel Fold,
# starts one app and reports. Usage:
#   $env:APP_IPA = "Bejeweled 2 1.5.5.ipa"; $env:EXTRA_ARGS = "--trace-messages"; .\agent_cycle.ps1
$ErrorActionPreference = "Continue"
$tools = "C:\Users\Ben\Downloads\Godot_v4.7.2-stable_mono_win64\iOS on android\tools"
$p = "/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/tools"
$s = "adb-37191FDHS00295-Fgy7Ho._adb-tls-connect._tcp"  # Pixel Fold (gen 1)
$d = "C:\Users\Ben\Downloads\Godot_v4.7.2-stable_mono_win64\iOS on android"
$t = "/sdcard/Android/data/org.touchhle.android/files"
$app = if ($env:APP_IPA) { $env:APP_IPA } else { "Bejeweled 2 1.5.5.ipa" }
$extra = if ($env:EXTRA_ARGS) { $env:EXTRA_ARGS } else { "" }

$out = wsl -d Ubuntu -u ben -- bash "$p/agent_build.sh" 2>&1 | Out-String
if ($out -notmatch "GRADLE_EXIT=0") {
    "BUILD FAILED"
    ($out -split "`n" | Select-String "error|e: |warning: unused|-->|^\s+\|" | Select-Object -First 40) -join "`n"
    return
}
"BUILD OK"
wsl -d Ubuntu -u ben -- cp /home/ben/touchHLE-agent/android/app/build/outputs/apk/release/app-release.apk "/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-agent.apk"
adb -s $s install -r "$d\touchHLE-agent.apk" 2>&1 | Select-Object -Last 1
adb -s $s shell am force-stop org.touchhle.android
adb -s $s shell "rm -f $t/touchHLE_log.txt"
adb -s $s shell "am start -n org.touchhle.android/org.touchhle.android.MainActivity --es app_path '$t/touchHLE_apps/$app' --ez compat true --es extra_args '$extra'" | Out-Null
Start-Sleep 25
"focus: " + ((adb -s $s shell "dumpsys window | grep mCurrentFocus") -replace '\s+', ' ')
adb -s $s shell "grep -n 'Panic' -A1 $t/touchHLE_log.txt | head -4"
adb -s $s shell "tail -n 3 $t/touchHLE_log.txt"
