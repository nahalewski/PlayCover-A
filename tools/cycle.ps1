# Build, install on the Pixel Fold, launch an app directly (no picker taps), and report.
# Usage: $env:APP_IPA = "AngryBirdsHD_1.5.0.ipa"; .\cycle.ps1   (default: FlappyBird_1.2.ipa)
$ErrorActionPreference = "Continue"
$sp = "C:\Users\Ben\AppData\Local\Temp\claude\C--Users-Ben-Downloads-Godot-v4-7-2-stable-mono-win64\c3041004-6d17-45c5-8a53-8ba8147cfd23\scratchpad"
$p = "/mnt/c/Users/Ben/AppData/Local/Temp/claude/C--Users-Ben-Downloads-Godot-v4-7-2-stable-mono-win64/c3041004-6d17-45c5-8a53-8ba8147cfd23/scratchpad"
$s = "192.168.0.51:36119"  # Pixel Fold (gen 1)
$d = "C:\Users\Ben\Downloads\Godot_v4.7.2-stable_mono_win64\iOS on android"
$t = "/sdcard/Android/data/org.touchhle.android/files"
$app = if ($env:APP_IPA) { $env:APP_IPA } else { "FlappyBird_1.2.ipa" }

$out = wsl -d Ubuntu -u ben -- bash "$p/gradle_build.sh" 2>&1 | Out-String
if ($out -notmatch "GRADLE_EXIT=0") {
    "BUILD FAILED"
    ($out -split "`n" | Select-String "error|e: |warning: unused|-->|^\s+\|" | Select-Object -First 40) -join "`n"
    return
}
"BUILD OK"
wsl -d Ubuntu -u ben -- cp /home/ben/touchHLE/android/app/build/outputs/apk/release/app-release.apk "/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-built.apk"
adb -s $s install -r "$d\touchHLE-built.apk" 2>&1 | Select-Object -Last 1
adb -s $s shell am force-stop org.touchhle.android
adb -s $s shell rm -f "$t/touchHLE_log.txt"
adb -s $s shell "am start -n org.touchhle.android/org.touchhle.android.MainActivity --es app_path '$t/touchHLE_apps/$app' --ez compat true --es extra_args '$env:EXTRA_ARGS'" | Out-Null
Start-Sleep 15
"focus: " + ((adb -s $s shell "dumpsys window | grep mCurrentFocus") -replace '\s+', ' ')
"pid: " + (adb -s $s shell pidof org.touchhle.android)
adb -s $s shell "grep -n 'Panic' -A1 $t/touchHLE_log.txt | head -4"
adb -s $s shell "tail -n 3 $t/touchHLE_log.txt"
