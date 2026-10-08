/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android;

import android.app.Activity;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.ViewGroup;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.RelativeLayout;

import java.io.File;
import java.io.FileInputStream;
import java.io.ByteArrayOutputStream;

/**
 * Shows emulated apps' UIWebViews with a real Android WebView laid over the
 * game surface. The emulator writes commands to webview_cmd.txt in the app's
 * external files directory:
 *   line 1: sequence number (a command is applied once per new number)
 *   line 2: show | html | hide
 *   line 3: x y w h (physical pixels of the game surface)
 *   line 4: base URL (html only, may be empty)
 *   rest:   URL (show) or HTML (html)
 */
final class WebOverlay {
    private static final String TAG = "AnastasisWebOverlay";
    private final Activity activity;
    private final ViewGroup layout;
    private final File file;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private WebView web;
    private long lastSeq = -1;

    WebOverlay(Activity activity, ViewGroup layout) {
        this.activity = activity;
        this.layout = layout;
        File dir = activity.getExternalFilesDir(null);
        this.file = dir == null ? null : new File(dir, "webview_cmd.txt");
        // A command left over from an earlier run must not replay.
        if (file != null && file.exists()) file.delete();
        handler.postDelayed(poll, 200);
    }

    private final Runnable poll = new Runnable() {
        @Override public void run() {
            try {
                check();
            } catch (Throwable t) {
                Log.w(TAG, "web overlay command failed", t);
            }
            handler.postDelayed(this, 150);
        }
    };

    private String readFile() throws Exception {
        try (FileInputStream in = new FileInputStream(file)) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[8192];
            int n;
            while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
            return out.toString("UTF-8");
        }
    }

    private void check() throws Exception {
        if (file == null || !file.exists()) return;
        String text = readFile();
        String[] lines = text.split("\n", 5);
        if (lines.length < 3) return;
        long seq = Long.parseLong(lines[0].trim());
        if (seq == lastSeq) return;
        lastSeq = seq;
        String command = lines[1].trim();
        if (command.equals("hide")) {
            hide();
            return;
        }
        String[] r = lines[2].trim().split(" ");
        int x = Integer.parseInt(r[0]), y = Integer.parseInt(r[1]);
        int w = Math.max(1, Integer.parseInt(r[2])), h = Math.max(1, Integer.parseInt(r[3]));
        String base = lines.length > 3 ? lines[3] : "";
        String payload = lines.length > 4 ? lines[4] : "";
        show(x, y, w, h);
        if (command.equals("html")) {
            web.loadDataWithBaseURL(base.isEmpty() ? null : base, payload, "text/html", "UTF-8", null);
        } else {
            web.loadUrl(payload.trim());
        }
    }

    private void show(int x, int y, int w, int h) {
        if (web == null) {
            web = new WebView(activity);
            WebSettings s = web.getSettings();
            s.setJavaScriptEnabled(true);
            // The game's web pages pick their native bridge from the browser
            // type; the emulated app is an iPhone app, so look like one.
            s.setUserAgentString("Mozilla/5.0 (iPhone; CPU iPhone OS 4_3 like Mac OS X) AppleWebKit/533.17.9 (KHTML, like Gecko) Version/5.0.2 Mobile/8F190 Safari/6533.18.5");
            s.setDomStorageEnabled(true);
            s.setLoadWithOverviewMode(true);
            s.setUseWideViewPort(true);
            s.setMixedContentMode(WebSettings.MIXED_CONTENT_ALWAYS_ALLOW);
            web.setWebViewClient(new WebViewClient() {
                @Override
                public android.webkit.WebResourceResponse shouldInterceptRequest(WebView v, android.webkit.WebResourceRequest req) {
                    Log.i(TAG, "request: " + req.getUrl() + " mainFrame=" + req.isForMainFrame());
                    return null;
                }
                @Override
                public boolean shouldOverrideUrlLoading(WebView v, android.webkit.WebResourceRequest req) {
                    String url = req.getUrl().toString();
                    String scheme = req.getUrl().getScheme();
                    Log.i(TAG, "navigation: " + url);
                    if ("http".equals(scheme) || "https".equals(scheme)) return false;
                    // Custom schemes are for the emulated app (e.g. a close button).
                    reportNavigation(url);
                    return true;
                }
            });
            web.setWebChromeClient(new android.webkit.WebChromeClient() {
                @Override
                public boolean onConsoleMessage(android.webkit.ConsoleMessage m) {
                    Log.i(TAG, "console: " + m.message() + " @" + m.sourceId() + ":" + m.lineNumber());
                    return true;
                }
            });
            web.setBackgroundColor(0xFFFFFFFF);
            layout.addView(web, params(x, y, w, h));
        } else {
            web.setLayoutParams(params(x, y, w, h));
        }
        web.setVisibility(android.view.View.VISIBLE);
    }

    private RelativeLayout.LayoutParams params(int x, int y, int w, int h) {
        RelativeLayout.LayoutParams p = new RelativeLayout.LayoutParams(w, h);
        p.leftMargin = x;
        p.topMargin = y;
        return p;
    }

    private void reportNavigation(String url) {
        if (file == null) return;
        try {
            File tmp = new File(file.getParentFile(), "webview_evt.tmp");
            try (java.io.FileOutputStream out = new java.io.FileOutputStream(tmp)) {
                out.write((System.nanoTime() + "\n" + url).getBytes("UTF-8"));
            }
            tmp.renameTo(new File(file.getParentFile(), "webview_evt.txt"));
        } catch (Exception e) {
            Log.w(TAG, "couldn't report navigation", e);
        }
    }

    private void hide() {
        if (web == null) return;
        layout.removeView(web);
        web.destroy();
        web = null;
    }
}
