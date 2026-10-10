/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android;

import android.app.Service;
import android.content.Intent;
import android.os.IBinder;
import android.os.Process;

/**
 * Runs in the game process (":game") while a game is running. When the user
 * closes the app (removes it from the recent apps list, or swipes away the game
 * window) Android only destroys the activities; the emulator's native thread
 * would keep running in the background. This service ends the game process
 * when that happens.
 */
public final class GameCleanupService extends Service {
    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        // Not restarted if the process is killed.
        return START_NOT_STICKY;
    }

    @Override
    public void onTaskRemoved(Intent rootIntent) {
        super.onTaskRemoved(rootIntent);
        Process.killProcess(Process.myPid());
    }
}
