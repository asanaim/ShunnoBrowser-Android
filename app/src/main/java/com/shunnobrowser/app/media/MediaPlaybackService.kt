package com.shunnobrowser.app.media

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.IBinder
import androidx.core.app.NotificationCompat

class MediaPlaybackService : Service() {
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val channelId = "shunno_media_bg"
        val mgr = getSystemService(NotificationManager::class.java)
        mgr.createNotificationChannel(
            NotificationChannel(channelId, "Background Media & Downloads", NotificationManager.IMPORTANCE_LOW)
        )
        val notif: Notification = NotificationCompat.Builder(this, channelId)
            .setContentTitle("Shunno Browser")
            .setContentText(intent?.getStringExtra("title") ?: "Background playback & multi-thread engine active")
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setOngoing(true)
            .build()
        startForeground(101, notif)
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null
}
