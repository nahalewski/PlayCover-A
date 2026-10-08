/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * Parts of this file are derived from SDL 2's Android project template, which
 * has a different license. Please see vendor/SDL/LICENSE.txt for details.
 */
package org.touchhle.android;

import java.util.ArrayList;

import org.libsdl.app.SDLActivity;

/**
 * A wrapper class over SDLActivity: the activity that hosts the emulator.
 *
 * It is normally started by {@link LauncherActivity}, which says which app to
 * run via intent extras. Empty launches return to the modern launcher.
 */
public class MainActivity extends SDLActivity {
    /** Intent extra (String): path of the .ipa or .app to run. */
    public static final String EXTRA_APP_PATH = "app_path";
    /** Intent extra (boolean): run with --ignore-unknown-selectors. */
    public static final String EXTRA_COMPAT = "compat";
    /** Closed launcher settings, separately from the developer extra_args override. */
    public static final String EXTRA_RUNTIME_OPTIONS = "runtime_options";
    private boolean exitingIPA;
    private boolean matchDeviceRotation = true; // follow the tablet, within the app's supported orientations
    private boolean naturalPortrait;
    private int physicalOrientation = -1;
    private android.view.OrientationEventListener rotationListener;
    private android.widget.ImageButton ipaMenuButton;
    private static native void nativeForceOrientation(int orientation);

    @Override
    protected void onCreate(android.os.Bundle state) {
        super.onCreate(state);
        String path = getIntent().getStringExtra(EXTRA_APP_PATH);
        if (path == null || path.trim().isEmpty()) {
            // A launcher shortcut or restored empty activity must never enter
            // SDL's legacy app picker. Finish before this surface is resumed.
            startActivity(new android.content.Intent(this, LauncherActivity.class)
                .addFlags(android.content.Intent.FLAG_ACTIVITY_CLEAR_TOP | android.content.Intent.FLAG_ACTIVITY_SINGLE_TOP));
            finish();
            return;
        }
        int rotation = getWindowManager().getDefaultDisplay().getRotation();
        boolean portrait = getResources().getConfiguration().orientation ==
            android.content.res.Configuration.ORIENTATION_PORTRAIT;
        naturalPortrait = ((rotation == 0 || rotation == 2) == portrait);
        updateForcedOrientation();
        installExitButton();
        if (mLayout != null) new WebOverlay(this, mLayout);
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    /** Android overlay, outside the emulated app's view hierarchy. */
    private void installExitButton() {
        if (mLayout == null) return;
        android.widget.ImageButton button = new android.widget.ImageButton(this);
        button.setImageResource(R.mipmap.ic_launcher);
        button.setScaleType(android.widget.ImageView.ScaleType.FIT_CENTER);
        button.setPadding(0, 0, 0, 0);
        button.setContentDescription("IPA menu");
        if (android.os.Build.VERSION.SDK_INT >= 26) button.setTooltipText("IPA menu");
        android.graphics.drawable.GradientDrawable background = new android.graphics.drawable.GradientDrawable();
        background.setShape(android.graphics.drawable.GradientDrawable.OVAL);
        background.setColor(android.graphics.Color.rgb(26, 29, 36));
        button.setBackground(background);
        button.setClipToOutline(true);
        button.setElevation(dp(6));
        android.widget.RelativeLayout.LayoutParams params =
            new android.widget.RelativeLayout.LayoutParams(dp(44), dp(44));
        params.addRule(android.widget.RelativeLayout.ALIGN_PARENT_LEFT);
        params.addRule(android.widget.RelativeLayout.ALIGN_PARENT_TOP);
        params.leftMargin = dp(8);
        params.topMargin = dp(8);
        mLayout.addView(button, params);
        ipaMenuButton = button;
        button.setOnApplyWindowInsetsListener((view, insets) -> {
            int left = insets.getSystemWindowInsetLeft();
            int top = insets.getSystemWindowInsetTop();
            if (android.os.Build.VERSION.SDK_INT >= 28 && insets.getDisplayCutout() != null) {
                left = Math.max(left, insets.getDisplayCutout().getSafeInsetLeft());
                top = Math.max(top, insets.getDisplayCutout().getSafeInsetTop());
            }
            params.leftMargin = dp(8) + left;
            params.topMargin = dp(8) + top;
            view.setLayoutParams(params);
            return insets;
        });
        button.requestApplyInsets();
        button.setOnClickListener(view -> showIPAMenu());
    }

    /** Manual orientation: -1 = automatic, else 0 portrait, 1/3 landscape, 2 upside down. */
    private int manualOrientation = -1;

    private void showIPAMenu() {
        String[] names = {"Automatic (follow device)", "Portrait", "Landscape (left)", "Landscape (right)", "Portrait upside down"};
        int[] values = {-1, 0, 3, 1, 2};
        int checked = 0;
        for (int i = 0; i < values.length; i++) if (values[i] == manualOrientation) checked = i;
        // Material 3 (dark) tonal palette, drawn by hand: no Material library here.
        final int surface = android.graphics.Color.argb(0xD9, 0x2B, 0x29, 0x30); // ~85% opaque
        final int onSurface = 0xFFE6E0E9;
        final int onSurfaceVariant = 0xFFCAC4D0;
        final int primary = 0xFFD0BCFF;
        final int selectedContainer = android.graphics.Color.argb(0xCC, 0x4A, 0x44, 0x58);
        final int error = 0xFFF2B8B5;

        android.app.Dialog dialog = new android.app.Dialog(this, android.R.style.Theme_DeviceDefault_Dialog_NoActionBar);
        android.widget.LinearLayout panel = new android.widget.LinearLayout(this);
        panel.setOrientation(android.widget.LinearLayout.VERTICAL);
        panel.setPadding(dp(8), dp(20), dp(8), dp(12));
        android.graphics.drawable.GradientDrawable panelBg = new android.graphics.drawable.GradientDrawable();
        panelBg.setColor(surface);
        panelBg.setCornerRadius(dp(28));
        panel.setBackground(panelBg);

        android.widget.TextView title = new android.widget.TextView(this);
        title.setText("Screen orientation");
        title.setTextColor(onSurface);
        title.setTextSize(22);
        title.setPadding(dp(16), 0, dp(16), dp(4));
        panel.addView(title);
        android.widget.TextView subtitle = new android.widget.TextView(this);
        subtitle.setText("For games that don't rotate by themselves");
        subtitle.setTextColor(onSurfaceVariant);
        subtitle.setTextSize(14);
        subtitle.setPadding(dp(16), 0, dp(16), dp(12));
        panel.addView(subtitle);

        for (int i = 0; i < names.length; i++) {
            final int value = values[i];
            final boolean selected = (i == checked);
            android.widget.TextView row = new android.widget.TextView(this);
            row.setText((selected ? "✓  " : "     ") + names[i]);
            row.setTextSize(16);
            row.setTextColor(selected ? primary : onSurface);
            row.setGravity(android.view.Gravity.CENTER_VERTICAL);
            row.setPadding(dp(16), 0, dp(16), 0);
            android.graphics.drawable.GradientDrawable rowBg = new android.graphics.drawable.GradientDrawable();
            rowBg.setCornerRadius(dp(28));
            rowBg.setColor(selected ? selectedContainer : android.graphics.Color.TRANSPARENT);
            row.setBackground(new android.graphics.drawable.RippleDrawable(
                android.content.res.ColorStateList.valueOf(android.graphics.Color.argb(0x33, 0xD0, 0xBC, 0xFF)), rowBg, null));
            row.setOnClickListener(v -> {
                manualOrientation = value;
                matchDeviceRotation = manualOrientation < 0;
                applyManualOrientation();
                dialog.dismiss();
            });
            android.widget.LinearLayout.LayoutParams rowParams =
                new android.widget.LinearLayout.LayoutParams(android.view.ViewGroup.LayoutParams.MATCH_PARENT, dp(52));
            rowParams.setMargins(dp(8), dp(2), dp(8), dp(2));
            panel.addView(row, rowParams);
        }

        android.widget.LinearLayout buttons = new android.widget.LinearLayout(this);
        buttons.setGravity(android.view.Gravity.END);
        buttons.setPadding(dp(8), dp(12), dp(8), 0);
        android.widget.TextView exit = menuButton("Exit IPA", error);
        exit.setOnClickListener(v -> { dialog.dismiss(); exitCurrentIPA(); });
        android.widget.TextView close = menuButton("Close", primary);
        close.setOnClickListener(v -> dialog.dismiss());
        buttons.addView(exit);
        buttons.addView(close);
        panel.addView(buttons);

        dialog.setContentView(panel);
        android.view.Window window = dialog.getWindow();
        if (window != null) {
            window.setBackgroundDrawable(new android.graphics.drawable.ColorDrawable(android.graphics.Color.TRANSPARENT));
            window.setDimAmount(0.25f); // keep the game visible behind the menu
            window.setLayout(Math.min(dp(360), getResources().getDisplayMetrics().widthPixels - dp(32)),
                android.view.ViewGroup.LayoutParams.WRAP_CONTENT);
        }
        dialog.show();
    }

    private android.widget.TextView menuButton(String label, int color) {
        android.widget.TextView button = new android.widget.TextView(this);
        button.setText(label);
        button.setTextColor(color);
        button.setTextSize(14);
        button.setAllCaps(false);
        button.setGravity(android.view.Gravity.CENTER);
        button.setPadding(dp(16), 0, dp(16), 0);
        button.setMinHeight(dp(40));
        android.graphics.drawable.GradientDrawable shape = new android.graphics.drawable.GradientDrawable();
        shape.setCornerRadius(dp(20));
        shape.setColor(android.graphics.Color.TRANSPARENT);
        button.setBackground(new android.graphics.drawable.RippleDrawable(
            android.content.res.ColorStateList.valueOf(android.graphics.Color.argb(0x33, 0xFF, 0xFF, 0xFF)), shape, null));
        return button;
    }

    /** Turn the Android window to match a manual choice, or release it again. */
    private void applyManualOrientation() {
        int requested;
        switch (manualOrientation) {
            case 0: requested = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_PORTRAIT; break;
            case 2: requested = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_REVERSE_PORTRAIT; break;
            case 3: requested = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE; break;
            case 1: requested = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_REVERSE_LANDSCAPE; break;
            default: requested = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR; break;
        }
        setRequestedOrientation(requested);
        updateForcedOrientation();
    }

    /**
     * Android already rotates the window to follow the device (unless the user
     * locked rotation), so the game only has to be shown upright inside that
     * window: portrait window -> portrait content, wide window -> landscape
     * content. Never ask for upside-down: the system has already turned the
     * window, and locking the activity to an orientation is what left the UI
     * stuck upside down.
     */
    private void updateForcedOrientation() {
        if (manualOrientation >= 0) {
            nativeForceOrientation(10 + manualOrientation);
            return;
        }
        if (!matchDeviceRotation) {
            nativeForceOrientation(-1);
            return;
        }
        android.util.DisplayMetrics metrics = getResources().getDisplayMetrics();
        int rotation = getWindowManager().getDefaultDisplay().getRotation();
        if (metrics.heightPixels >= metrics.widthPixels) {
            nativeForceOrientation(0);
        } else {
            // Which landscape: follows which way the window was turned.
            boolean turnedOne = naturalPortrait ? rotation == 1 : (rotation == 0 || rotation == 3);
            nativeForceOrientation(turnedOne ? 3 : 1);
        }
    }

    /** Aspect ratio of the game window, kept inside what Android allows for PiP. */
    private android.util.Rational pipAspectRatio() {
        android.util.DisplayMetrics metrics = getResources().getDisplayMetrics();
        double ratio = (double) metrics.widthPixels / Math.max(1, metrics.heightPixels);
        ratio = Math.max(0.42, Math.min(2.38, ratio));
        return new android.util.Rational((int) Math.round(ratio * 1000), 1000);
    }

    private void updatePictureInPictureParams(boolean autoEnter) {
        if (android.os.Build.VERSION.SDK_INT < 26) return;
        try {
            android.app.PictureInPictureParams.Builder builder = new android.app.PictureInPictureParams.Builder()
                .setAspectRatio(pipAspectRatio());
            // With gesture navigation Android 12+ only enters PiP on its own
            // if asked to; onUserLeaveHint covers button navigation.
            if (android.os.Build.VERSION.SDK_INT >= 31) builder.setAutoEnterEnabled(autoEnter);
            setPictureInPictureParams(builder.build());
        } catch (RuntimeException e) {
            // PiP is a convenience; never let it break the game.
        }
    }

    /** Leaving the game (Home, Recents, gestures) shrinks it to a floating
     *  picture-in-picture window; tapping that window brings it back. */
    @Override protected void onUserLeaveHint() {
        super.onUserLeaveHint();
        if (exitingIPA || android.os.Build.VERSION.SDK_INT < 26) return;
        if (!getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_PICTURE_IN_PICTURE)) return;
        try {
            enterPictureInPictureMode(new android.app.PictureInPictureParams.Builder()
                .setAspectRatio(pipAspectRatio()).build());
        } catch (RuntimeException e) {
            android.util.Log.w("AnastasisPiP", "enterPictureInPictureMode failed", e);
            // Not allowed right now (e.g. already in PiP): just background.
        }
    }

    @Override public void onPictureInPictureModeChanged(boolean inPip, android.content.res.Configuration config) {
        super.onPictureInPictureModeChanged(inPip, config);
        if (ipaMenuButton != null) {
            ipaMenuButton.setVisibility(inPip ? android.view.View.GONE : android.view.View.VISIBLE);
        }
    }

    @Override public void onConfigurationChanged(android.content.res.Configuration config) {
        super.onConfigurationChanged(config);
        updateForcedOrientation();
    }

    @Override protected void onResume() {
        super.onResume();
        updateForcedOrientation();
        updatePictureInPictureParams(true);
    }

    private boolean isOwnGameProcess() {
        String expected = getPackageName() + ":game";
        if (android.os.Build.VERSION.SDK_INT >= 28) {
            return expected.equals(android.app.Application.getProcessName());
        }
        android.app.ActivityManager manager =
            (android.app.ActivityManager) getSystemService(ACTIVITY_SERVICE);
        java.util.List<android.app.ActivityManager.RunningAppProcessInfo> processes = manager.getRunningAppProcesses();
        if (processes != null) for (android.app.ActivityManager.RunningAppProcessInfo process : processes) {
            if (process.pid == android.os.Process.myPid()) return expected.equals(process.processName);
        }
        return false;
    }

    private void exitCurrentIPA() {
        if (exitingIPA) return;
        exitingIPA = true;
        boolean ownGameProcess = isOwnGameProcess();
        final int gamePID = android.os.Process.myPid();
        startActivity(new android.content.Intent(this, LauncherActivity.class)
            .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK |
                android.content.Intent.FLAG_ACTIVITY_CLEAR_TOP | android.content.Intent.FLAG_ACTIVITY_SINGLE_TOP));
        if (ownGameProcess) {
            // SDL shutdown may join a blocked native emulation thread. Terminate
            // only this isolated process after the launcher request is delivered.
            // A worker remains responsive even if the UI enters SDL onDestroy.
            new Thread(() -> {
                android.os.SystemClock.sleep(250);
                android.os.Process.killProcess(gamePID);
            }, "Anastasis-exit-IPA").start();
        } else {
            finish();
        }
    }

    @Override
    protected void messageboxCreateAndShow(android.os.Bundle args) {
        String title = args.getString("title", "");
        if (!title.contains("crashed")) {
            super.messageboxCreateAndShow(args);
            return;
        }
        // File and IPA metadata reads stay off the UI thread.
        new Thread(() -> {
            String path = getIntent().getStringExtra(EXTRA_APP_PATH);
            String name = "Unknown IPA";
            if (path != null) {
                java.io.File ipa = new java.io.File(path);
                name = ipa.getName();
                IpaInfo info = IpaInfo.Companion.read(ipa);
                if (info != null && info.getDisplayName() != null && !info.getDisplayName().equals(name)) {
                    name = info.getDisplayName() + "\n" + name;
                }
            }
            String recent;
            try {
                recent = CrashLog.readTail(new java.io.File(getExternalFilesDir(null), "touchHLE_log.txt"), 64 * 1024);
            } catch (Exception error) {
                recent = "Recent runtime log is unavailable: " + error.getMessage();
            }
            final String appName = name;
            final String log = recent;
            runOnUiThread(() -> showCrashDialog(args, appName, log));
        }, "Anastasis-crash-log").start();
    }

    private void showCrashDialog(android.os.Bundle args, String appName, String log) {
        if (isFinishing() || isDestroyed()) {
            synchronized (messageboxSelection) { messageboxSelection.notifyAll(); }
            return;
        }
        android.widget.LinearLayout content = new android.widget.LinearLayout(this);
        content.setOrientation(android.widget.LinearLayout.VERTICAL);
        int padding = (int) (16 * getResources().getDisplayMetrics().density);
        content.setPadding(padding, padding, padding, padding);
        android.widget.TextView name = new android.widget.TextView(this);
        name.setText(appName); name.setTextSize(18);
        content.addView(name);
        android.widget.TextView details = new android.widget.TextView(this);
        details.setText(args.getString("message", "").replace("touchHLE crashed", "Anastasis crashed") +
            "\n\nRecent runtime log\n\n" + log);
        details.setTypeface(android.graphics.Typeface.MONOSPACE);
        details.setTextSize(12); details.setTextIsSelectable(true);
        android.widget.ScrollView scroll = new android.widget.ScrollView(this);
        scroll.addView(details);
        content.addView(scroll, new android.widget.LinearLayout.LayoutParams(-1,
            (int) (getResources().getDisplayMetrics().heightPixels * 0.5f)));
        android.app.AlertDialog dialog = new android.app.AlertDialog.Builder(this)
            .setTitle("Anastasis crashed").setView(content).create();
        dialog.setCancelable(false);
        dialog.setButton(android.app.AlertDialog.BUTTON_POSITIVE, "Close", (unused, which) -> messageboxSelection[0] = 1);
        dialog.setButton(android.app.AlertDialog.BUTTON_NEUTRAL, "Open log directory", (unused, which) -> messageboxSelection[0] = 0);
        dialog.setOnDismissListener(unused -> {
            synchronized (messageboxSelection) { messageboxSelection.notifyAll(); }
        });
        dialog.show();
    }

    @Override
    protected String[] getLibraries() {
        return new String[]{
            "SDL2",
            "touchHLE"
        };
    }

    private static boolean validReportedIosVersion(String setting) {
        final String prefix = "--reported-ios-version=";
        if (!setting.startsWith(prefix)) return false;
        String version = setting.substring(prefix.length());
        if (!version.matches("[0-9]{1,3}\\.[0-9]{1,3}(?:\\.[0-9]{1,3})?")) return false;
        String[] parts = version.split("\\.");
        for (int i = 0; i < parts.length; i++) {
            int value = Integer.parseInt(parts[i]);
            if (value > 255 || (i == 0 && value == 0) || (parts[i].length() > 1 && parts[i].startsWith("0"))) return false;
        }
        return true;
    }

    /** These become touchHLE's command-line arguments (after the program name). */
    @Override
    protected String[] getArguments() {
        ArrayList<String> arguments = new ArrayList<>();
        String appPath = getIntent().getStringExtra(EXTRA_APP_PATH);
        if (appPath != null) {
            arguments.add(appPath);
        }
        if (getIntent().getBooleanExtra(EXTRA_COMPAT, false)) {
            arguments.add("--ignore-unknown-selectors");
        }
        String[] settings = getIntent().getStringArrayExtra(EXTRA_RUNTIME_OPTIONS);
        if (settings != null) {
            for (String setting : settings) {
                if (setting != null && (setting.matches("--scale-hack=[1-4]") || setting.matches("--fps-limit=[0-9]{1,3}") ||
                    setting.equals("--upside-down") || setting.equals("--landscape-left") ||
                    setting.equals("--landscape-right") || setting.equals("--allow-network-access") ||
                    setting.equals("--disable-analog-stick-tilt-controls") || validReportedIosVersion(setting))) {
                    arguments.add(setting);
                }
            }
        }
        // Debugging aid: `adb shell am start ... --es extra_args "--trace-messages"`.
        String extraArgs = getIntent().getStringExtra("extra_args");
        if (extraArgs != null) {
            for (String arg : extraArgs.split(" ")) {
                if (!arg.isEmpty()) {
                    arguments.add(arg);
                }
            }
        }
        return arguments.toArray(new String[0]);
    }
}
