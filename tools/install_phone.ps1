# Install the latest WSL-built APK (org.touchhle.android) on the Pixel 9 Pro Fold (Ben tests by hand) and report.
# The Samsung install happens in tab32.ps1. Run this after every new build.
$sp = "C:\Users\Ben\AppData\Local\Temp\claude\C--Users-Ben-Downloads-Godot-v4-7-2-stable-mono-win64\c3041004-6d17-45c5-8a53-8ba8147cfd23\scratchpad"
wsl -d Ubuntu -u ben bash -c "cp ~/touchHLE/android/app/build/outputs/apk/release/app-release.apk /mnt/c/Users/Ben/AppData/Local/Temp/claude/C--Users-Ben-Downloads-Godot-v4-7-2-stable-mono-win64/c3041004-6d17-45c5-8a53-8ba8147cfd23/scratchpad/tab32.apk"
$phone = "192.168.0.22:41105"
if ((adb devices) -match [regex]::Escape($phone)) {
    "Pixel 9 Pro Fold: " + ((adb -s $phone install -r "$sp\tab32.apk" 2>&1 | Select-Object -Last 1) -join "")
} else {
    "Pixel 9 Pro Fold NOT connected at $phone (check adb devices / wireless debugging port)"
}
