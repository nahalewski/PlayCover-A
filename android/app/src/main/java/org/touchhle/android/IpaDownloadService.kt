package org.touchhle.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper

/** A foreground notification keeps concurrent transfers independent of activity recreation. */
class IpaDownloadService : Service() {
    private val handler = Handler(Looper.getMainLooper())
    private val listener: () -> Unit = { refresh() }
    private val ticker = object : Runnable {
        override fun run() { refresh(); handler.postDelayed(this, 2000) }
    }
    override fun onCreate() {
        super.onCreate()
        if (Build.VERSION.SDK_INT >= 26) (getSystemService(NOTIFICATION_SERVICE) as NotificationManager)
            .createNotificationChannel(NotificationChannel(CHANNEL, "IPA downloads", NotificationManager.IMPORTANCE_LOW))
        startForeground(7, notification("Preparing downloads"))
        IpaDownloads.addListener(listener)
        handler.post(ticker)
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        IpaDownloads.schedule(applicationContext)
        refresh()
        return START_STICKY
    }
    private fun notification(text: String): Notification {
        val launch = PendingIntent.getActivity(this, 0, Intent(this, LauncherActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or if (Build.VERSION.SDK_INT >= 23) PendingIntent.FLAG_IMMUTABLE else 0)
        val builder = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this, CHANNEL) else Notification.Builder(this)
        return builder.setSmallIcon(android.R.drawable.stat_sys_download).setContentTitle("PlayCover-A downloads")
            .setContentText(text).setContentIntent(launch).setOngoing(true).setOnlyAlertOnce(true).build()
    }
    private fun refresh() {
        val jobs = IpaDownloads.snapshot(applicationContext).filter { it.status == "RUNNING" || it.status == "QUEUED" }
        if (!IpaDownloads.active()) { stopForeground(true); stopSelf(); return }
        val downloaded = jobs.sumOf { it.received } / (1024 * 1024)
        (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).notify(7, notification("${jobs.size} downloads · $downloaded MiB received"))
    }
    override fun onDestroy() {
        handler.removeCallbacksAndMessages(null); IpaDownloads.removeListener(listener); super.onDestroy()
    }
    override fun onBind(intent: Intent?): IBinder? = null
    private companion object { const val CHANNEL = "ipa_downloads" }
}
