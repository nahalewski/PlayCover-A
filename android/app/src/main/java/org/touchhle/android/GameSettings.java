/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android;

import android.content.Context;
import android.content.SharedPreferences;
import android.util.Log;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * Per-game settings of the in-game menu, keyed by the app's bundle id.
 *
 * Emulator options live in the emulator's own per-app options file
 * (touchHLE_options.txt, one "bundle.id: --opt --opt" line per app), which the
 * emulator reads at launch. Only the flags managed here are added or removed;
 * everything else in the file is kept as it is. Android-only settings (FPS
 * counter, manual orientation, menu button) are kept in SharedPreferences.
 *
 * Settings that can change while the game runs are also sent to the emulator
 * through settings_cmd.txt (sequence number, then key=value lines).
 */
final class GameSettings {
    private static final String TAG = "AnastasisGameSettings";
    static final String OPTIONS_FILE = "touchHLE_options.txt";
    static final String VIEW_DEFAULT = "default", VIEW_STRETCH = "stretch", VIEW_16_9 = "16:9", VIEW_BLUR = "blur";
    private static final String PREFS = "game_settings";

    final String bundleId;
    private final File dir;
    private final SharedPreferences prefs;

    // Emulator options (touchHLE_options.txt).
    String view = VIEW_DEFAULT;
    int scale = 1;          // --scale-hack=N, 1 = default
    int fpsLimit = 0;       // --fps-limit=N, 0 = default
    boolean blockAds = true;
    boolean network = true;
    boolean unlockStorePurchases = false;
    String reportedAppVersion = null;
    // Android-only (SharedPreferences).
    boolean fpsCounter;
    int manualOrientation = -1;
    boolean fadedMenuButton;

    private GameSettings(Context context, String bundleId) {
        this.bundleId = bundleId;
        this.dir = context.getExternalFilesDir(null);
        this.prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    /** Reads the current settings of this game. */
    static GameSettings load(Context context, String bundleId) {
        GameSettings s = new GameSettings(context, bundleId);
        for (String option : appOptions(s.dir, bundleId)) {
            if (option.equals("--widescreen")) s.view = VIEW_STRETCH;
            else if (option.equals("--widescreen=blur")) s.view = VIEW_BLUR;
            else if (option.equals("--widescreen=16:9")) s.view = VIEW_16_9;
            else if (option.startsWith("--scale-hack=")) s.scale = parseInt(option.substring(13), 1);
            else if (option.startsWith("--fps-limit=")) s.fpsLimit = parseInt(option.substring(12), 0);
            else if (option.equals("--allow-ads")) s.blockAds = false;
            else if (option.equals("--no-network-access")) s.network = false;
            else if (option.equals("--unlock-store-purchases")) s.unlockStorePurchases = true;
            else if (option.startsWith("--reported-app-version=")) s.reportedAppVersion = option.substring(23);
        }
        if (s.reportedAppVersion == null && bundleId != null && bundleId.toLowerCase().contains("zenoniaonline")) {
            s.reportedAppVersion = "2.10.0";
        }
        s.fpsCounter = s.prefs.getBoolean(bundleId + ".fps_counter", false);
        s.manualOrientation = s.prefs.getInt(bundleId + ".orientation", -1);
        s.fadedMenuButton = s.prefs.getBoolean(bundleId + ".faded_menu_button", false);
        return s;
    }

    private static int parseInt(String text, int fallback) {
        try {
            return Integer.parseInt(text.trim());
        } catch (NumberFormatException e) {
            return fallback;
        }
    }

    void savePrefs() {
        prefs.edit()
            .putBoolean(bundleId + ".fps_counter", fpsCounter)
            .putInt(bundleId + ".orientation", manualOrientation)
            .putBoolean(bundleId + ".faded_menu_button", fadedMenuButton)
            .apply();
    }

    // ---- Bundle id ----

    private static String cacheKey(String appPath) {
        File file = new File(appPath);
        return "bundle_id:" + file.getAbsolutePath() + "@" + file.lastModified();
    }

    /** The bundle id remembered from an earlier run of this exact file, or null. */
    static String cachedBundleId(Context context, String appPath) {
        if (appPath == null) return null;
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(cacheKey(appPath), null);
    }

    /** Reads the bundle id from the IPA (slow: call off the UI thread). */
    static String resolveBundleId(Context context, String appPath) {
        String cached = cachedBundleId(context, appPath);
        if (cached != null) return cached;
        File file = new File(appPath);
        String id = null;
        if (file.isFile()) {
            try {
                IpaInfo info = IpaInfo.Companion.read(file);
                if (info != null) id = info.getBundleId();
            } catch (Throwable t) {
                Log.w(TAG, "couldn't read the bundle id of " + appPath, t);
            }
        }
        if (id == null || id.trim().isEmpty()) id = file.getName();
        id = id.trim();
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(cacheKey(appPath), id).apply();
        return id;
    }

    // ---- Options file ----

    private static String readText(File file) {
        try (FileInputStream in = new FileInputStream(file)) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[8192];
            int n;
            while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
            return out.toString("UTF-8");
        } catch (Exception e) {
            return null;
        }
    }

    private static void writeAtomically(File file, String text) throws Exception {
        String name = file.getName();
        int dot = name.lastIndexOf('.');
        File tmp = new File(file.getParentFile(), (dot > 0 ? name.substring(0, dot) : name) + ".tmp");
        try (FileOutputStream out = new FileOutputStream(tmp)) {
            out.write(text.getBytes("UTF-8"));
        }
        if (!tmp.renameTo(file)) {
            throw new java.io.IOException("rename " + tmp + " failed");
        }
    }

    /** The app id of an options-file line, or null for comments and empty lines. */
    private static String lineAppId(String line) {
        String content = line;
        int hash = content.indexOf('#');
        if (hash >= 0) content = content.substring(0, hash);
        int colon = content.indexOf(':');
        if (colon < 0) return null;
        return content.substring(0, colon).trim();
    }

    /** The options on this app's line of the options file. */
    static List<String> appOptions(File dir, String bundleId) {
        List<String> result = new ArrayList<>();
        if (dir == null) return result;
        String text = readText(new File(dir, OPTIONS_FILE));
        if (text == null) return result;
        for (String line : text.split("\n", -1)) {
            if (!bundleId.equals(lineAppId(line))) continue;
            String content = line;
            int hash = content.indexOf('#');
            if (hash >= 0) content = content.substring(0, hash);
            content = content.substring(content.indexOf(':') + 1).trim();
            if (!content.isEmpty()) result.addAll(Arrays.asList(content.split("\\s+")));
        }
        return result;
    }

    private static boolean isManaged(String option) {
        return option.startsWith("--widescreen") || option.startsWith("--scale-hack=") ||
            option.startsWith("--fps-limit=") || option.equals("--allow-ads") ||
            option.equals("--no-network-access") || option.equals("--unlock-store-purchases") ||
            option.startsWith("--reported-app-version=");
    }

    /** The flags the current settings need (nothing for defaults). */
    private List<String> managedOptions() {
        List<String> options = new ArrayList<>();
        switch (view) {
            case VIEW_STRETCH: options.add("--widescreen"); break;
            case VIEW_16_9: options.add("--widescreen=16:9"); break;
            case VIEW_BLUR: options.add("--widescreen=blur"); break;
            default: break;
        }
        if (scale > 1) options.add("--scale-hack=" + scale);
        if (fpsLimit > 0) options.add("--fps-limit=" + fpsLimit);
        if (!blockAds) options.add("--allow-ads");
        if (!network) options.add("--no-network-access");
        if (unlockStorePurchases) options.add("--unlock-store-purchases");
        if (reportedAppVersion != null && !reportedAppVersion.trim().isEmpty()) {
            options.add("--reported-app-version=" + reportedAppVersion.trim());
        }
        return options;
    }

    /**
     * Rewrites this app's line of the options file: other apps' lines, comments
     * and unknown flags on this line stay as they are.
     */
    void saveOptions() {
        if (dir == null) return;
        File file = new File(dir, OPTIONS_FILE);
        String text = readText(file);
        if (text == null) text = "";
        List<String> lines = new ArrayList<>(Arrays.asList(text.split("\n", -1)));
        int index = -1;
        for (int i = 0; i < lines.size(); i++) {
            if (bundleId.equals(lineAppId(lines.get(i)))) { index = i; break; }
        }
        List<String> kept = new ArrayList<>();
        String comment = "";
        if (index >= 0) {
            String line = lines.get(index);
            if (line.endsWith("\r")) line = line.substring(0, line.length() - 1);
            int hash = line.indexOf('#');
            if (hash >= 0) { comment = " " + line.substring(hash); line = line.substring(0, hash); }
            String content = line.substring(line.indexOf(':') + 1).trim();
            for (String option : content.isEmpty() ? new String[0] : content.split("\\s+")) {
                if (isManaged(option)) continue;
                // Network turned off: an explicit "allow" would contradict it.
                if (!network && option.equals("--allow-network-access")) continue;
                kept.add(option);
            }
        }
        kept.addAll(managedOptions());
        String newLine = kept.isEmpty() ? null : bundleId + ": " + android.text.TextUtils.join(" ", kept) + comment;
        if (index >= 0) {
            if (newLine == null && comment.isEmpty()) lines.remove(index);
            else lines.set(index, newLine != null ? newLine : comment.trim());
        } else if (newLine != null) {
            // Append, keeping the file's final newline.
            if (!lines.isEmpty() && lines.get(lines.size() - 1).isEmpty()) lines.add(lines.size() - 1, newLine);
            else { lines.add(newLine); lines.add(""); }
        } else {
            return;
        }
        try {
            writeAtomically(file, android.text.TextUtils.join("\n", lines));
        } catch (Exception e) {
            Log.w(TAG, "couldn't write " + file, e);
        }
    }

    // ---- Live changes ----

    /** Tells the running emulator about the settings that apply immediately. */
    void sendLiveCommand() {
        if (dir == null) return;
        String text = System.currentTimeMillis() + "\n" +
            "view=" + view + "\n" +
            "ads=" + (blockAds ? "block" : "allow") + "\n" +
            "network=" + (network ? "on" : "off") + "\n" +
            "store=" + (unlockStorePurchases ? "unlock" : "normal") + "\n" +
            "app_version=" + (reportedAppVersion != null ? reportedAppVersion : "") + "\n";
        try {
            writeAtomically(new File(dir, "settings_cmd.txt"), text);
        } catch (Exception e) {
            Log.w(TAG, "couldn't write settings_cmd.txt", e);
        }
    }
}
