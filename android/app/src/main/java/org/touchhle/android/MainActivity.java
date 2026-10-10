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
        // Lets the game end when the app is closed from the recent apps list.
        try {
            startService(new android.content.Intent(this, GameCleanupService.class));
        } catch (RuntimeException e) {
            // Only a convenience; never stop the game over it.
        }
        String path = getIntent().getStringExtra(EXTRA_APP_PATH);
        if (path == null || path.trim().isEmpty()) {
            // A launcher shortcut or restored empty activity must never enter
            // SDL's legacy app picker. Finish before this surface is resumed.
            startActivity(new android.content.Intent(this, LauncherActivity.class)
                .addFlags(android.content.Intent.FLAG_ACTIVITY_CLEAR_TOP | android.content.Intent.FLAG_ACTIVITY_SINGLE_TOP));
            finish();
            return;
        }
        hideSystemBars();
        int rotation = getWindowManager().getDefaultDisplay().getRotation();
        boolean portrait = getResources().getConfiguration().orientation ==
            android.content.res.Configuration.ORIENTATION_PORTRAIT;
        naturalPortrait = ((rotation == 0 || rotation == 2) == portrait);
        updateForcedOrientation();
        installExitButton();
        if (mLayout != null) {
            new WebOverlay(this, mLayout);
            new MovieOverlay(this, mLayout);
            new AlertOverlay(this, mLayout);
            fpsOverlay = new FpsOverlay(this, mLayout);
        }
        loadGameSettings(path);
    }

    // ---- Per-game settings ----

    private GameSettings gameSettings;
    private FpsOverlay fpsOverlay;
    private boolean menuWanted;
    /** Values the emulator was started with: changing these needs a restart. */
    private int startedScale, startedFpsLimit;

    /** The bundle id needs the IPA to be read: do it off the UI thread. */
    private void loadGameSettings(String path) {
        new Thread(() -> {
            String id = GameSettings.resolveBundleId(this, path);
            GameSettings settings = GameSettings.load(this, id);
            runOnUiThread(() -> {
                if (isFinishing() || isDestroyed()) return;
                gameSettings = settings;
                startedScale = settings.scale;
                startedFpsLimit = settings.fpsLimit;
                if (settings.manualOrientation >= 0) {
                    manualOrientation = settings.manualOrientation;
                    matchDeviceRotation = false;
                    applyManualOrientation();
                }
                if (fpsOverlay != null) fpsOverlay.setEnabled(settings.fpsCounter);
                applyMenuButtonStyle();
                // The emulator skips the first command it sees (it may be left
                // over from an earlier run); send the current state now so later
                // changes from the menu are not the ones skipped.
                settings.sendLiveCommand();
                if (menuWanted) { menuWanted = false; showIPAMenu(); }
            });
        }, "Anastasis-game-settings").start();
    }

    private void applyMenuButtonStyle() {
        if (ipaMenuButton != null && gameSettings != null) {
            ipaMenuButton.setAlpha(gameSettings.fadedMenuButton ? 0.35f : 1f);
        }
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    /**
     * Immersive full-screen mode: hides the status bar and the navigation bar.
     * A swipe from the screen edge shows them temporarily and they hide again.
     */
    private void hideSystemBars() {
        android.view.Window window = getWindow();
        if (window == null) return;
        // SDL sets this when its (non-fullscreen) window is created.
        window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_FORCE_NOT_FULLSCREEN);
        window.addFlags(android.view.WindowManager.LayoutParams.FLAG_FULLSCREEN);
        if (android.os.Build.VERSION.SDK_INT >= 28) {
            android.view.WindowManager.LayoutParams attributes = window.getAttributes();
            attributes.layoutInDisplayCutoutMode =
                android.view.WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            window.setAttributes(attributes);
        }
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false);
            android.view.WindowInsetsController controller = window.getInsetsController();
            if (controller != null) {
                controller.hide(android.view.WindowInsets.Type.statusBars()
                    | android.view.WindowInsets.Type.navigationBars());
                controller.setSystemBarsBehavior(
                    android.view.WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            window.getDecorView().setSystemUiVisibility(
                android.view.View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                | android.view.View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                | android.view.View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                | android.view.View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                | android.view.View.SYSTEM_UI_FLAG_FULLSCREEN
                | android.view.View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY);
        }
    }

    /**
     * SDL resets the window style to "bars visible" when it creates its
     * window, and a swipe from the edge shows the bars: hide them again.
     */
    @Override
    public void onSystemUiVisibilityChange(int visibility) {
        super.onSystemUiVisibilityChange(visibility);
        boolean barsHidden = (visibility & android.view.View.SYSTEM_UI_FLAG_FULLSCREEN) != 0
            && (visibility & android.view.View.SYSTEM_UI_FLAG_HIDE_NAVIGATION) != 0;
        if (!barsHidden) {
            getWindow().getDecorView().post(this::hideSystemBars);
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        // Bars come back after dialogs, the keyboard or the app switcher.
        if (hasFocus) hideSystemBars();
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

    // Material 3 (dark) tonal palette, drawn by hand: no Material library here.
    private static final int M3_SURFACE = 0xD92B2930; // ~85% opaque
    private static final int M3_ON_SURFACE = 0xFFE6E0E9;
    private static final int M3_ON_SURFACE_VARIANT = 0xFFCAC4D0;
    private static final int M3_PRIMARY = 0xFFD0BCFF;
    private static final int M3_ON_PRIMARY = 0xFF381E72;
    private static final int M3_OUTLINE = 0xFF938F99;
    private static final int M3_OUTLINE_VARIANT = 0x6649454F;
    private static final int M3_SECONDARY_CONTAINER = 0xFF4A4458;
    private static final int M3_ON_SECONDARY_CONTAINER = 0xFFE8DEF8;
    private static final int M3_ERROR = 0xFFF2B8B5;

    private android.app.Dialog menuDialog;

    private interface Choice { void chosen(int index); }
    private interface Toggle { void toggled(boolean on); }

    /** Lays out its children left to right, wrapping into more rows. */
    private static final class FlowLayout extends android.view.ViewGroup {
        private final int gap;
        FlowLayout(android.content.Context context, int gap) { super(context); this.gap = gap; }

        @Override protected void onMeasure(int widthSpec, int heightSpec) {
            int maxWidth = MeasureSpec.getSize(widthSpec);
            boolean bounded = MeasureSpec.getMode(widthSpec) != MeasureSpec.UNSPECIFIED;
            int x = 0, y = 0, rowHeight = 0, widest = 0;
            for (int i = 0; i < getChildCount(); i++) {
                android.view.View child = getChildAt(i);
                if (child.getVisibility() == GONE) continue;
                child.measure(MeasureSpec.makeMeasureSpec(maxWidth, MeasureSpec.AT_MOST),
                    MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED));
                int w = child.getMeasuredWidth(), h = child.getMeasuredHeight();
                if (bounded && x > 0 && x + w > maxWidth) { x = 0; y += rowHeight + gap; rowHeight = 0; }
                x += w + gap;
                widest = Math.max(widest, x - gap);
                rowHeight = Math.max(rowHeight, h);
            }
            setMeasuredDimension(bounded ? maxWidth : widest, y + rowHeight);
        }

        @Override protected void onLayout(boolean changed, int l, int t, int r, int b) {
            int maxWidth = r - l;
            int x = 0, y = 0, rowHeight = 0;
            for (int i = 0; i < getChildCount(); i++) {
                android.view.View child = getChildAt(i);
                if (child.getVisibility() == GONE) continue;
                int w = child.getMeasuredWidth(), h = child.getMeasuredHeight();
                if (x > 0 && x + w > maxWidth) { x = 0; y += rowHeight + gap; rowHeight = 0; }
                child.layout(x, y, x + w, y + h);
                x += w + gap;
                rowHeight = Math.max(rowHeight, h);
            }
        }
    }

    private android.widget.TextView text(String value, int color, float size) {
        android.widget.TextView view = new android.widget.TextView(this);
        view.setText(value);
        view.setTextColor(color);
        view.setTextSize(size);
        return view;
    }

    private static android.graphics.drawable.Drawable ripple(android.graphics.drawable.GradientDrawable shape) {
        return new android.graphics.drawable.RippleDrawable(
            android.content.res.ColorStateList.valueOf(android.graphics.Color.argb(0x33, 0xD0, 0xBC, 0xFF)), shape, null);
    }

    private void styleChip(android.widget.TextView chip, String label, boolean selected) {
        chip.setText(selected ? "✓  " + label : label);
        chip.setTextColor(selected ? M3_ON_SECONDARY_CONTAINER : M3_ON_SURFACE);
        android.graphics.drawable.GradientDrawable shape = new android.graphics.drawable.GradientDrawable();
        shape.setCornerRadius(dp(8));
        if (selected) {
            shape.setColor(M3_SECONDARY_CONTAINER);
        } else {
            shape.setColor(android.graphics.Color.TRANSPARENT);
            shape.setStroke(Math.max(1, dp(1)), M3_OUTLINE);
        }
        chip.setBackground(ripple(shape));
        chip.setSelected(selected);
    }

    /** Single-choice filter chips; selected < 0 selects none. */
    private android.view.View chipGroup(String[] labels, int selected, Choice choice) {
        FlowLayout group = new FlowLayout(this, dp(8));
        android.widget.TextView[] chips = new android.widget.TextView[labels.length];
        for (int i = 0; i < labels.length; i++) {
            final int index = i;
            android.widget.TextView chip = text(labels[i], M3_ON_SURFACE, 14);
            chip.setGravity(android.view.Gravity.CENTER);
            chip.setMinHeight(dp(36));
            chip.setPadding(dp(14), 0, dp(14), 0);
            styleChip(chip, labels[i], i == selected);
            chip.setOnClickListener(v -> {
                for (int j = 0; j < chips.length; j++) styleChip(chips[j], labels[j], j == index);
                choice.chosen(index);
            });
            chips[i] = chip;
            group.addView(chip, new android.view.ViewGroup.LayoutParams(
                android.view.ViewGroup.LayoutParams.WRAP_CONTENT, dp(36)));
        }
        return group;
    }

    private android.view.View toggleRow(String label, boolean on, Toggle toggle) {
        android.widget.LinearLayout row = new android.widget.LinearLayout(this);
        row.setOrientation(android.widget.LinearLayout.HORIZONTAL);
        row.setGravity(android.view.Gravity.CENTER_VERTICAL);
        row.setMinimumHeight(dp(40));
        android.widget.TextView name = text(label, M3_ON_SURFACE, 15);
        row.addView(name, new android.widget.LinearLayout.LayoutParams(0,
            android.view.ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        // Material 3 switch, drawn by hand (the platform Switch looks different
        // on every vendor theme, and is just "ON"/"OFF" text on some).
        android.widget.FrameLayout track = new android.widget.FrameLayout(this);
        android.view.View thumb = new android.view.View(this);
        track.addView(thumb);
        final boolean[] state = {on};
        final Runnable paint = () -> {
            boolean checked = state[0];
            android.graphics.drawable.GradientDrawable trackShape = new android.graphics.drawable.GradientDrawable();
            trackShape.setCornerRadius(dp(16));
            trackShape.setColor(checked ? M3_PRIMARY : 0xFF36343B);
            if (!checked) trackShape.setStroke(dp(2), M3_OUTLINE);
            track.setBackground(trackShape);
            android.graphics.drawable.GradientDrawable thumbShape = new android.graphics.drawable.GradientDrawable();
            thumbShape.setShape(android.graphics.drawable.GradientDrawable.OVAL);
            thumbShape.setColor(checked ? M3_ON_PRIMARY : M3_OUTLINE);
            thumb.setBackground(thumbShape);
            int size = checked ? dp(24) : dp(16);
            android.widget.FrameLayout.LayoutParams p = new android.widget.FrameLayout.LayoutParams(size, size,
                android.view.Gravity.CENTER_VERTICAL | (checked ? android.view.Gravity.END : android.view.Gravity.START));
            p.leftMargin = p.rightMargin = checked ? dp(4) : dp(8);
            thumb.setLayoutParams(p);
            track.setContentDescription(label + (checked ? ", on" : ", off"));
        };
        paint.run();
        row.addView(track, new android.widget.LinearLayout.LayoutParams(dp(52), dp(32)));
        row.setOnClickListener(v -> {
            state[0] = !state[0];
            paint.run();
            toggle.toggled(state[0]);
        });
        track.setOnClickListener(v -> row.performClick());
        return row;
    }

    /** A titled group of controls; wide menus put the title beside them. */
    private void addSection(android.widget.LinearLayout body, boolean wide, String title, String note,
                            android.view.View... controls) {
        if (body.getChildCount() > 0) {
            android.view.View divider = new android.view.View(this);
            divider.setBackgroundColor(M3_OUTLINE_VARIANT);
            android.widget.LinearLayout.LayoutParams p = new android.widget.LinearLayout.LayoutParams(
                android.view.ViewGroup.LayoutParams.MATCH_PARENT, Math.max(1, dp(1)));
            p.setMargins(dp(16), dp(10), dp(16), dp(10));
            body.addView(divider, p);
        }
        android.widget.LinearLayout section = new android.widget.LinearLayout(this);
        section.setOrientation(wide ? android.widget.LinearLayout.HORIZONTAL : android.widget.LinearLayout.VERTICAL);
        section.setPadding(dp(16), 0, dp(16), 0);
        android.widget.TextView label = text(title, M3_PRIMARY, 14);
        label.setTypeface(android.graphics.Typeface.create("sans-serif-medium", android.graphics.Typeface.NORMAL));
        label.setPadding(0, wide ? dp(9) : 0, dp(8), wide ? 0 : dp(8));
        section.addView(label, wide
            ? new android.widget.LinearLayout.LayoutParams(dp(120), android.view.ViewGroup.LayoutParams.WRAP_CONTENT)
            : new android.widget.LinearLayout.LayoutParams(android.view.ViewGroup.LayoutParams.MATCH_PARENT,
                android.view.ViewGroup.LayoutParams.WRAP_CONTENT));
        android.widget.LinearLayout column = new android.widget.LinearLayout(this);
        column.setOrientation(android.widget.LinearLayout.VERTICAL);
        for (android.view.View control : controls) {
            android.widget.LinearLayout.LayoutParams p = new android.widget.LinearLayout.LayoutParams(
                android.view.ViewGroup.LayoutParams.MATCH_PARENT, android.view.ViewGroup.LayoutParams.WRAP_CONTENT);
            if (column.getChildCount() > 0) p.topMargin = dp(6);
            column.addView(control, p);
        }
        if (note != null) {
            android.widget.TextView hint = text(note, M3_ON_SURFACE_VARIANT, 12);
            hint.setPadding(0, dp(6), 0, 0);
            column.addView(hint);
        }
        section.addView(column, wide
            ? new android.widget.LinearLayout.LayoutParams(0, android.view.ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
            : new android.widget.LinearLayout.LayoutParams(android.view.ViewGroup.LayoutParams.MATCH_PARENT,
                android.view.ViewGroup.LayoutParams.WRAP_CONTENT));
        body.addView(section);
    }

    private static int indexOf(int[] values, int value) {
        for (int i = 0; i < values.length; i++) if (values[i] == value) return i;
        return -1;
    }

    private void showIPAMenu() {
        if (gameSettings == null) {
            // Still reading the game's bundle id; open as soon as it is known.
            menuWanted = true;
            android.widget.Toast.makeText(this, "Loading game settings…", android.widget.Toast.LENGTH_SHORT).show();
            return;
        }
        if (menuDialog != null && menuDialog.isShowing()) return;
        final GameSettings s = gameSettings;
        android.util.DisplayMetrics metrics = getResources().getDisplayMetrics();
        final int width = Math.min(dp(640), metrics.widthPixels - dp(32));
        final boolean wide = width >= dp(480);
        final int maxHeight = Math.round(metrics.heightPixels * 0.92f);

        android.app.Dialog dialog = new android.app.Dialog(this, android.R.style.Theme_DeviceDefault_Dialog_NoActionBar);
        menuDialog = dialog;
        android.widget.LinearLayout panel = new android.widget.LinearLayout(this);
        panel.setOrientation(android.widget.LinearLayout.VERTICAL);
        panel.setPadding(dp(8), dp(18), dp(8), dp(10));
        android.graphics.drawable.GradientDrawable panelBg = new android.graphics.drawable.GradientDrawable();
        panelBg.setColor(M3_SURFACE);
        panelBg.setCornerRadius(dp(28));
        panel.setBackground(panelBg);

        android.widget.TextView title = text("Game settings", M3_ON_SURFACE, 22);
        title.setPadding(dp(16), 0, dp(16), dp(2));
        panel.addView(title);
        android.widget.TextView subtitle = text("Saved for " + s.bundleId, M3_ON_SURFACE_VARIANT, 13);
        subtitle.setSingleLine(true);
        subtitle.setEllipsize(android.text.TextUtils.TruncateAt.MIDDLE);
        subtitle.setPadding(dp(16), 0, dp(16), dp(10));
        panel.addView(subtitle);

        android.widget.LinearLayout body = new android.widget.LinearLayout(this);
        body.setOrientation(android.widget.LinearLayout.VERTICAL);
        body.setPadding(0, dp(4), 0, dp(8));

        // Footer pieces first: settings that need a restart update them.
        android.widget.TextView restartNote = text("Restart to apply resolution / FPS limit", M3_ON_SURFACE_VARIANT, 12);
        android.widget.TextView restart = menuButton("Restart game", M3_ON_PRIMARY);
        android.graphics.drawable.GradientDrawable restartShape = new android.graphics.drawable.GradientDrawable();
        restartShape.setCornerRadius(dp(20));
        restartShape.setColor(M3_PRIMARY);
        restart.setBackground(ripple(restartShape));
        final Runnable updateRestart = () -> {
            boolean pending = s.scale != startedScale || s.fpsLimit != startedFpsLimit;
            restart.setVisibility(pending ? android.view.View.VISIBLE : android.view.View.GONE);
            restartNote.setVisibility(pending && wide ? android.view.View.VISIBLE : android.view.View.GONE);
        };

        // 1. View
        final String[] views = {GameSettings.VIEW_DEFAULT, GameSettings.VIEW_STRETCH, GameSettings.VIEW_BLUR, GameSettings.VIEW_16_9};
        int viewIndex = java.util.Arrays.asList(views).indexOf(s.view);
        addSection(body, wide, "View", "Generated keeps the picture sharp and fills the sides",
            chipGroup(new String[]{"Default", "Widescreen", "Widescreen generated", "16:9"}, viewIndex, i -> {
                s.view = views[i];
                s.saveOptions();
                s.sendLiveCommand();
            }));

        // 2. Rotation
        final int[] rotations = {-1, 0, 3, 1, 2};
        addSection(body, wide, "Rotation", null,
            chipGroup(new String[]{"Automatic", "Portrait", "Landscape left", "Landscape right", "Upside down"},
                Math.max(0, indexOf(rotations, manualOrientation)), i -> {
                    manualOrientation = rotations[i];
                    matchDeviceRotation = manualOrientation < 0;
                    s.manualOrientation = manualOrientation;
                    s.savePrefs();
                    applyManualOrientation();
                }));

        // 3. Network
        addSection(body, wide, "Network", null,
            chipGroup(new String[]{"On (default)", "Off"}, s.network ? 0 : 1, i -> {
                s.network = i == 0;
                s.saveOptions();
                s.sendLiveCommand();
            }));

        // 4. Resolution upscaler
        final int[] scales = {1, 2, 3, 4};
        addSection(body, wide, "Resolution", "Internal render scale · applies after a restart",
            chipGroup(new String[]{"Default (1x)", "2x", "3x", "4x"}, indexOf(scales, s.scale), i -> {
                s.scale = scales[i];
                s.saveOptions();
                updateRestart.run();
            }));

        // 5. Performance
        final int[] limits = {0, 30, 60};
        android.widget.TextView limitLabel = text("FPS limit", M3_ON_SURFACE, 15);
        limitLabel.setPadding(0, dp(4), 0, dp(2));
        addSection(body, wide, "Performance", "FPS limit applies after a restart",
            toggleRow("FPS counter", s.fpsCounter, on -> {
                s.fpsCounter = on;
                s.savePrefs();
                if (fpsOverlay != null) fpsOverlay.setEnabled(on);
            }),
            limitLabel,
            chipGroup(new String[]{"Default", "30", "60"}, indexOf(limits, s.fpsLimit), i -> {
                s.fpsLimit = limits[i];
                s.saveOptions();
                updateRestart.run();
            }));

        // 6. Ads
        addSection(body, wide, "Ads", null,
            toggleRow("Block ads", s.blockAds, on -> {
                s.blockAds = on;
                s.saveOptions();
                s.sendLiveCommand();
            }));

        // 7. Store Purchases & DLC
        addSection(body, wide, "Store / Content Unlock", "Auto-grant offline in-game store purchases & DLC",
            toggleRow("Unlock store purchases (Free items)", s.unlockStorePurchases, on -> {
                s.unlockStorePurchases = on;
                s.saveOptions();
                s.sendLiveCommand();
            }));

        // 8. App Version Spoofing
        String curVer = s.reportedAppVersion != null ? s.reportedAppVersion : "Default";
        addSection(body, wide, "Version Spoofing", "Spoofs reported app version to bypass online checks (e.g. 2.10.0 for Zenonia S)",
            chipGroup(new String[]{"Default", "2.10.0 (Zenonia S)", "Latest"},
                curVer.equals("2.10.0") ? 1 : (curVer.equals("Default") ? 0 : 2), i -> {
                    if (i == 0) s.reportedAppVersion = null;
                    else if (i == 1) s.reportedAppVersion = "2.10.0";
                    else s.reportedAppVersion = "2.10.0";
                    s.saveOptions();
                    s.sendLiveCommand();
                }));

        // 9. Menu button
        addSection(body, wide, "Menu button", null,
            toggleRow("Faded while playing", s.fadedMenuButton, on -> {
                s.fadedMenuButton = on;
                s.savePrefs();
                applyMenuButtonStyle();
            }));

        // The settings scroll; title and buttons stay in place.
        android.widget.ScrollView scroll = new android.widget.ScrollView(this) {
            @Override protected void onMeasure(int widthSpec, int heightSpec) {
                int available = maxHeight - title.getMeasuredHeight() - subtitle.getMeasuredHeight() - dp(96);
                super.onMeasure(widthSpec, MeasureSpec.makeMeasureSpec(Math.max(dp(120), available), MeasureSpec.AT_MOST));
            }
        };
        scroll.setVerticalFadingEdgeEnabled(true);
        scroll.setFadingEdgeLength(dp(16));
        scroll.addView(body);
        panel.addView(scroll, new android.widget.LinearLayout.LayoutParams(
            android.view.ViewGroup.LayoutParams.MATCH_PARENT, android.view.ViewGroup.LayoutParams.WRAP_CONTENT));

        android.widget.LinearLayout buttons = new android.widget.LinearLayout(this);
        buttons.setGravity(android.view.Gravity.END | android.view.Gravity.CENTER_VERTICAL);
        buttons.setPadding(dp(8), dp(8), dp(8), 0);
        restartNote.setPadding(dp(8), 0, dp(8), 0);
        buttons.addView(restartNote, new android.widget.LinearLayout.LayoutParams(0,
            android.view.ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        restart.setOnClickListener(v -> { dialog.dismiss(); restartGame(); });
        android.widget.TextView exit = menuButton("Exit IPA", M3_ERROR);
        exit.setOnClickListener(v -> { dialog.dismiss(); exitCurrentIPA(); });
        android.widget.TextView close = menuButton("Close", M3_PRIMARY);
        close.setOnClickListener(v -> dialog.dismiss());
        buttons.addView(restart);
        buttons.addView(exit);
        buttons.addView(close);
        panel.addView(buttons);
        updateRestart.run();

        dialog.setContentView(panel);
        dialog.setOnDismissListener(d -> { if (menuDialog == dialog) menuDialog = null; });
        android.view.Window window = dialog.getWindow();
        if (window != null) {
            window.setBackgroundDrawable(new android.graphics.drawable.ColorDrawable(android.graphics.Color.TRANSPARENT));
            window.setDimAmount(0.25f); // keep the game visible behind the menu
            window.setLayout(width, android.view.ViewGroup.LayoutParams.WRAP_CONTENT);
        }
        dialog.show();
    }

    /**
     * Starts this game again, so that options read only at launch (render
     * scale, FPS limit) take effect. The emulator can't be restarted inside its
     * process: queue a fresh launch with the same extras, then end the process
     * before the launch can be delivered to it, so Android starts a new one.
     */
    private void restartGame() {
        if (exitingIPA) return;
        exitingIPA = true;
        android.content.Intent intent = new android.content.Intent(this, MainActivity.class);
        android.os.Bundle extras = getIntent().getExtras();
        if (extras != null) intent.putExtras(extras);
        intent.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK | android.content.Intent.FLAG_ACTIVITY_CLEAR_TASK);
        startActivity(intent);
        android.os.Process.killProcess(android.os.Process.myPid());
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
        if (fpsOverlay != null) fpsOverlay.setPictureInPicture(inPip);
    }

    @Override public void onConfigurationChanged(android.content.res.Configuration config) {
        super.onConfigurationChanged(config);
        updateForcedOrientation();
        // The menu was sized for the old window shape: lay it out again.
        if (menuDialog != null && menuDialog.isShowing()) {
            menuDialog.dismiss();
            getWindow().getDecorView().postDelayed(this::showIPAMenu, 300);
        }
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
        // Command-line options win over the options file, so a launcher-wide
        // value would hide this game's own choice from the in-game menu.
        boolean gameScale = false, gameFpsLimit = false, gameNoNetwork = false;
        String bundleId = GameSettings.cachedBundleId(this, appPath);
        if (bundleId != null) {
            for (String option : GameSettings.appOptions(getExternalFilesDir(null), bundleId)) {
                if (option.startsWith("--scale-hack=")) gameScale = true;
                else if (option.startsWith("--fps-limit=")) gameFpsLimit = true;
                else if (option.equals("--no-network-access")) gameNoNetwork = true;
            }
        }
        if (settings != null) {
            for (String setting : settings) {
                if (setting == null || (gameScale && setting.startsWith("--scale-hack=")) ||
                    (gameFpsLimit && setting.startsWith("--fps-limit=")) ||
                    (gameNoNetwork && setting.equals("--allow-network-access"))) continue;
                if (setting != null && (setting.matches("--scale-hack=[1-4]") || setting.matches("--fps-limit=[0-9]{1,3}") ||
                    setting.equals("--upside-down") || setting.equals("--landscape-left") ||
                    setting.equals("--landscape-right") || setting.equals("--allow-network-access") ||
                    setting.equals("--disable-analog-stick-tilt-controls") ||
                    setting.startsWith("--a64-cache-") || setting.equals("--no-error-popup") ||
                    validReportedIosVersion(setting))) {
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
