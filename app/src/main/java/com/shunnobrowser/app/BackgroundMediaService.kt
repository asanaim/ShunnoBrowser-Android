package com.shunnobrowser.app

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

class BackgroundMediaService : Service() {

    companion object {
        const val CHANNEL_ID = "shunnobrowser_bg_media"
        const val NOTIF_ID = 2001
        const val EXTRA_TEXT = "extra_text"

        fun updateOrStart(context: Context, statusText: String) {
            val intent = Intent(context, BackgroundMediaService::class.java).apply {
                putExtra(EXTRA_TEXT, statusText)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, BackgroundMediaService::class.java))
        }
    }

    override fun onCreate() {
        super.onCreate()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val ch = NotificationChannel(
                CHANNEL_ID,
                "Shunno Browser Background Media & Downloads",
                NotificationManager.IMPORTANCE_LOW
            )
            getSystemService(NotificationManager::class.java).createNotificationChannel(ch)
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val text = intent?.getStringExtra(EXTRA_TEXT) ?: "Background audio & downloads active"
        val openIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        val notif = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle("Shunno Browser")
            .setContentText(text)
            .setContentIntent(openIntent)
            .setOngoing(true)
            .build()

        startForeground(NOTIF_ID, notif)
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null
}
