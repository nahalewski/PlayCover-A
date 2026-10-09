/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android;

import android.app.Activity;
import android.media.MediaPlayer;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.MotionEvent;
import android.view.ViewGroup;
import android.widget.RelativeLayout;
import android.widget.VideoView;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;

/**
 * Plays emulated apps' movies (MPMoviePlayerController) with the platform's
 * video decoder, in a VideoView laid over the game surface. The emulator
 * writes commands to movie_cmd.txt in the app's external files directory:
 *   line 1: sequence number (a command is applied once per new number)
 *   line 2: play | stop
 *   line 3: x y w h (physical pixels of the game surface)
 *   line 4: path of the movie file (play only)
 * and the end of the movie is reported in movie_evt.txt: the sequence number
 * of the play command, then done | user (the player tapped the movie to skip
 * it) | error.
 */
final class MovieOverlay {
    private static final String TAG = "AnastasisMovieOverlay";
    private final Activity activity;
    private final ViewGroup layout;
    private final File cmdFile;
    private final File dir;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private VideoView video;
    private long lastSeq = -1;
    private long playingSeq = -1;

    MovieOverlay(Activity activity, ViewGroup layout) {
        this.activity = activity;
        this.layout = layout;
        this.dir = activity.getExternalFilesDir(null);
        this.cmdFile = dir == null ? null : new File(dir, "movie_cmd.txt");
        // Commands and events left over from an earlier run must not replay.
        if (cmdFile != null && cmdFile.exists()) cmdFile.delete();
        if (dir != null) new File(dir, "movie_evt.txt").delete();
        handler.postDelayed(poll, 200);
    }

    private final Runnable poll = new Runnable() {
        @Override public void run() {
            try {
                check();
            } catch (Throwable t) {
                Log.w(TAG, "movie overlay command failed", t);
            }
            handler.postDelayed(this, 100);
        }
    };

    private String readFile() throws Exception {
        try (FileInputStream in = new FileInputStream(cmdFile)) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[4096];
            int n;
            while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
            return out.toString("UTF-8");
        }
    }

    private void check() throws Exception {
        if (cmdFile == null || !cmdFile.exists()) return;
        String[] lines = readFile().split("\n", 4);
        if (lines.length < 3) return;
        long seq = Long.parseLong(lines[0].trim());
        if (seq == lastSeq) return;
        lastSeq = seq;
        String command = lines[1].trim();
        if (command.equals("stop")) {
            // The emulated app stopped the movie itself: nothing to report.
            playingSeq = -1;
            remove();
            return;
        }
        if (!command.equals("play") || lines.length < 4) return;
        String[] r = lines[2].trim().split(" ");
        int x = Integer.parseInt(r[0]), y = Integer.parseInt(r[1]);
        int w = Math.max(1, Integer.parseInt(r[2])), h = Math.max(1, Integer.parseInt(r[3]));
        play(seq, x, y, w, h, lines[3].trim());
    }

    private void play(final long seq, int x, int y, int w, int h, String path) {
        remove();
        playingSeq = seq;
        video = new VideoView(activity);
        // The game draws into its own SurfaceView; the movie's surface has to be
        // above it or the movie is hidden behind the game's (black) frame.
        video.setZOrderOnTop(true);
        video.setBackgroundColor(0xFF000000);
        video.setOnPreparedListener(new MediaPlayer.OnPreparedListener() {
            @Override public void onPrepared(MediaPlayer mp) {
                mp.setLooping(false);
            }
        });
        video.setOnCompletionListener(new MediaPlayer.OnCompletionListener() {
            @Override public void onCompletion(MediaPlayer mp) {
                finish(seq, "done");
            }
        });
        video.setOnErrorListener(new MediaPlayer.OnErrorListener() {
            @Override public boolean onError(MediaPlayer mp, int what, int extra) {
                Log.w(TAG, "playback error " + what + "/" + extra);
                finish(seq, "error");
                return true;
            }
        });
        // iOS lets the player tap through a movie to skip it.
        video.setOnTouchListener(new android.view.View.OnTouchListener() {
            @Override public boolean onTouch(android.view.View v, MotionEvent e) {
                if (e.getAction() == MotionEvent.ACTION_UP) finish(seq, "user");
                return true;
            }
        });
        RelativeLayout.LayoutParams p = new RelativeLayout.LayoutParams(w, h);
        p.leftMargin = x;
        p.topMargin = y;
        layout.addView(video, p);
        video.setVideoPath(path);
        video.start();
    }

    private void finish(long seq, String how) {
        if (seq != playingSeq) return;
        playingSeq = -1;
        remove();
        if (dir == null) return;
        try {
            File tmp = new File(dir, "movie_evt.tmp");
            try (java.io.FileOutputStream out = new java.io.FileOutputStream(tmp)) {
                out.write((seq + "\n" + how).getBytes("UTF-8"));
            }
            tmp.renameTo(new File(dir, "movie_evt.txt"));
        } catch (Exception e) {
            Log.w(TAG, "couldn't report the end of the movie", e);
        }
    }

    private void remove() {
        if (video == null) return;
        try {
            video.stopPlayback();
        } catch (Throwable ignored) {
        }
        layout.removeView(video);
        video = null;
    }
}
