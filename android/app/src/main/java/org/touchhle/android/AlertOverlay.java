/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android;

import android.app.Activity;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;

/**
 * Shows emulated apps' UIAlertViews as a card laid over the game surface (a
 * view, not a dialog window, so the game surface keeps window focus). The
 * emulator writes alert_cmd.txt in the app's external files directory:
 *   line 1: sequence number (a command is applied once per new number)
 *   line 2: show | hide
 *   line 3: title      (percent-escaped: %25 = %, %0A = newline, %0D = CR)
 *   line 4: message    (same escaping)
 *   line 5: number of buttons N
 *   next N lines: button titles (same escaping)
 * and the tapped button is reported in alert_evt.txt: "sequence\nindex".
 */
final class AlertOverlay {
    private static final String TAG = "AnastasisAlertOverlay";
    private final Activity activity;
    private final ViewGroup layout;
    private final File file;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private View shown;
    private long lastSeq = -1;
    private long currentSeq = -1;

    AlertOverlay(Activity activity, ViewGroup layout) {
        this.activity = activity;
        this.layout = layout;
        File dir = activity.getExternalFilesDir(null);
        this.file = dir == null ? null : new File(dir, "alert_cmd.txt");
        // A command left over from an earlier run must not replay.
        if (file != null && file.exists()) file.delete();
        handler.postDelayed(poll, 200);
    }

    private final Runnable poll = new Runnable() {
        @Override public void run() {
            try {
                check();
            } catch (Throwable t) {
                Log.w(TAG, "alert overlay command failed", t);
            }
            handler.postDelayed(this, 100);
        }
    };

    private static String unescape(String s) {
        return s.replace("%0A", "\n").replace("%0D", "\r").replace("%25", "%");
    }

    private String readFile() throws Exception {
        try (FileInputStream in = new FileInputStream(file)) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[4096];
            int n;
            while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
            return out.toString("UTF-8");
        }
    }

    private void check() throws Exception {
        if (file == null || !file.exists()) return;
        String[] lines = readFile().split("\n", -1);
        if (lines.length < 2) return;
        long seq = Long.parseLong(lines[0].trim());
        if (seq == lastSeq) return;
        lastSeq = seq;
        hide();
        if (!lines[1].trim().equals("show") || lines.length < 5) return;
        int count = Integer.parseInt(lines[4].trim());
        String[] buttons = new String[count];
        for (int i = 0; i < count && 5 + i < lines.length; i++) buttons[i] = unescape(lines[5 + i]);
        currentSeq = seq;
        show(unescape(lines[2]), unescape(lines[3]), buttons);
    }

    private int dp(int v) {
        return Math.round(v * activity.getResources().getDisplayMetrics().density);
    }

    private void show(String title, String message, String[] buttons) {
        FrameLayout scrim = new FrameLayout(activity);
        scrim.setBackgroundColor(0x99000000);
        scrim.setClickable(true); // swallow touches meant for the game underneath

        LinearLayout card = new LinearLayout(activity);
        card.setOrientation(LinearLayout.VERTICAL);
        GradientDrawable bg = new GradientDrawable();
        bg.setColor(0xFF2B2B30);
        bg.setCornerRadius(dp(14));
        card.setBackground(bg);
        card.setPadding(dp(20), dp(18), dp(20), dp(14));

        if (!title.isEmpty()) {
            TextView t = new TextView(activity);
            t.setText(title);
            t.setTextColor(Color.WHITE);
            t.setTextSize(19);
            t.setTypeface(Typeface.DEFAULT_BOLD);
            t.setGravity(Gravity.CENTER);
            card.addView(t, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        }
        if (!message.isEmpty()) {
            TextView m = new TextView(activity);
            m.setText(message);
            m.setTextColor(0xFFDDDDDD);
            m.setTextSize(15);
            m.setGravity(Gravity.CENTER);
            m.setPadding(0, dp(10), 0, dp(14));
            ScrollView sv = new ScrollView(activity);
            sv.addView(m);
            card.addView(sv, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        }

        // Two buttons sit side by side, like iOS; more are stacked.
        LinearLayout row = new LinearLayout(activity);
        row.setOrientation(buttons.length == 2 ? LinearLayout.HORIZONTAL : LinearLayout.VERTICAL);
        for (int i = 0; i < buttons.length; i++) {
            final int index = i;
            Button b = new Button(activity);
            b.setText(buttons[i] == null ? "" : buttons[i]);
            b.setAllCaps(false);
            b.setOnClickListener(v -> {
                if (currentSeq < 0) return;
                long seq = currentSeq;
                currentSeq = -1;
                hide();
                reportButton(seq, index);
            });
            LinearLayout.LayoutParams lp = buttons.length == 2
                ? new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
                : new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            lp.setMargins(dp(4), dp(4), dp(4), dp(4));
            row.addView(b, lp);
        }
        card.addView(row, new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));

        int width = Math.min(dp(420), (int) (layout.getWidth() * 0.9f));
        if (width <= 0) width = dp(360);
        FrameLayout.LayoutParams cp = new FrameLayout.LayoutParams(width, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.CENTER);
        scrim.addView(card, cp);
        layout.addView(scrim, new android.widget.RelativeLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        shown = scrim;
    }

    private void hide() {
        if (shown != null) {
            layout.removeView(shown);
            shown = null;
        }
    }

    private void reportButton(long seq, int index) {
        if (file == null) return;
        try {
            File tmp = new File(file.getParentFile(), "alert_evt.tmp");
            try (FileOutputStream out = new FileOutputStream(tmp)) {
                out.write((seq + "\n" + index).getBytes("UTF-8"));
            }
            tmp.renameTo(new File(file.getParentFile(), "alert_evt.txt"));
        } catch (Exception e) {
            Log.w(TAG, "couldn't report alert button", e);
        }
    }
}
