/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android;

import android.app.Activity;
import android.os.Handler;
import android.os.Looper;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.RelativeLayout;
import android.widget.TextView;

import java.io.File;
import java.io.FileInputStream;

/**
 * A small frames-per-second label in the top-right corner. The emulator writes
 * fps.txt (one number) about once a second while the game renders; it is only
 * polled while the label is shown.
 */
final class FpsOverlay {
    private final Activity activity;
    private final ViewGroup layout;
    private final File file;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private TextView label;
    private boolean enabled;
    private boolean hiddenForPip;

    FpsOverlay(Activity activity, ViewGroup layout) {
        this.activity = activity;
        this.layout = layout;
        File dir = activity.getExternalFilesDir(null);
        this.file = dir == null ? null : new File(dir, "fps.txt");
    }

    private int dp(int value) {
        return Math.round(value * activity.getResources().getDisplayMetrics().density);
    }

    private final Runnable poll = new Runnable() {
        @Override public void run() {
            if (!enabled) return;
            String fps = read();
            label.setText(fps == null ? "-- FPS" : fps + " FPS");
            handler.postDelayed(this, 500);
        }
    };

    private String read() {
        if (file == null || !file.exists()) return null;
        // Stale numbers (the game stopped presenting) are not shown.
        if (System.currentTimeMillis() - file.lastModified() > 3000) return null;
        try (FileInputStream in = new FileInputStream(file)) {
            byte[] buf = new byte[32];
            int n = in.read(buf);
            if (n <= 0) return null;
            String text = new String(buf, 0, n, "UTF-8").trim();
            return text.matches("[0-9]{1,4}(\\.[0-9])?") ? text : null;
        } catch (Exception e) {
            return null;
        }
    }

    void setEnabled(boolean on) {
        if (on == enabled) return;
        enabled = on;
        if (on) {
            if (label == null) create();
            updateVisibility();
            handler.removeCallbacks(poll);
            handler.post(poll);
        } else {
            handler.removeCallbacks(poll);
            if (label != null) label.setVisibility(View.GONE);
        }
    }

    void setPictureInPicture(boolean inPip) {
        hiddenForPip = inPip;
        updateVisibility();
    }

    private void updateVisibility() {
        if (label != null) label.setVisibility(enabled && !hiddenForPip ? View.VISIBLE : View.GONE);
    }

    private void create() {
        label = new TextView(activity);
        label.setTextColor(0xFFFFFFFF);
        label.setTextSize(12);
        label.setTypeface(android.graphics.Typeface.MONOSPACE, android.graphics.Typeface.BOLD);
        label.setGravity(Gravity.CENTER);
        label.setPadding(dp(8), dp(3), dp(8), dp(3));
        label.setText("-- FPS");
        android.graphics.drawable.GradientDrawable bg = new android.graphics.drawable.GradientDrawable();
        bg.setColor(0x8C000000); // semi-transparent black
        bg.setCornerRadius(dp(10));
        label.setBackground(bg);
        label.setElevation(dp(4));
        label.setClickable(false);
        label.setFocusable(false);
        RelativeLayout.LayoutParams params = new RelativeLayout.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        params.addRule(RelativeLayout.ALIGN_PARENT_RIGHT);
        params.addRule(RelativeLayout.ALIGN_PARENT_TOP);
        params.rightMargin = dp(8);
        params.topMargin = dp(8);
        layout.addView(label, params);
        label.setOnApplyWindowInsetsListener((view, insets) -> {
            int right = insets.getSystemWindowInsetRight();
            int top = insets.getSystemWindowInsetTop();
            if (android.os.Build.VERSION.SDK_INT >= 28 && insets.getDisplayCutout() != null) {
                right = Math.max(right, insets.getDisplayCutout().getSafeInsetRight());
                top = Math.max(top, insets.getDisplayCutout().getSafeInsetTop());
            }
            params.rightMargin = dp(8) + right;
            params.topMargin = dp(8) + top;
            view.setLayoutParams(params);
            return insets;
        });
        label.requestApplyInsets();
    }
}
