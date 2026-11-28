package com.company.odoonativeprint.worker

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import com.company.odoonativeprint.R

/**
 * Keeps active print jobs alive during screen-off or app switching.
 * Compliant with Android 14/15 (API 34/35): uses ServiceCompat.startForeground with
 * FOREGROUND_SERVICE_TYPE_DATA_SYNC and implements onTimeout() to stop cleanly.
 */
class PrintForegroundService : Service() {

    override fun onCreate() {
        super.onCreate()
        ensureChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val activeCount = intent?.getIntExtra(EXTRA_ACTIVE_COUNT, 0) ?: 0
        val started = startForegroundCompat(activeCount.coerceAtLeast(0))
        if (!started) {
            isForeground = false
            stopSelfResult(startId)
            return START_NOT_STICKY
        }
        isForeground = true
        if (activeCount <= 0 || !desiredActive) {
            isForeground = false
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelfResult(startId)
        }
        return START_NOT_STICKY
    }

    private fun startForegroundCompat(activeCount: Int): Boolean {
        val notification = buildNotification(activeCount)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            val tryConnected = runCatching {
                ServiceCompat.startForeground(
                    this,
                    NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
                )
            }
            if (tryConnected.isSuccess) return true
        }
        val fallbackType = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        } else {
            0
        }
        return runCatching {
            ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, fallbackType)
        }.isSuccess
    }

    override fun onTimeout(startId: Int, fgsType: Int) {
        isForeground = false
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelfResult(startId)
    }

    override fun onDestroy() {
        isForeground = false
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val mgr = getSystemService(NotificationManager::class.java)
            val channel = NotificationChannel(
                CHANNEL_ID,
                getString(R.string.notification_channel_printing),
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                setShowBadge(false)
            }
            mgr.createNotificationChannel(channel)
        }
    }

    private fun buildNotification(activeCount: Int): Notification =
        NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_itbaa_logo)
            .setContentTitle(getString(R.string.notification_printing_title))
            .setContentText(getString(R.string.notification_printing_active, activeCount))
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()

    companion object {
        private const val CHANNEL_ID = "itbaa_silent_printing"
        private const val NOTIFICATION_ID = 4107
        private const val EXTRA_ACTIVE_COUNT = "active_count"

        @Volatile
        private var isForeground: Boolean = false

        @Volatile
        private var desiredActive: Boolean = false

        fun sync(context: Context, activeCount: Int) {
            val intent = Intent(context, PrintForegroundService::class.java)
                .putExtra(EXTRA_ACTIVE_COUNT, activeCount)
            runCatching {
                if (activeCount > 0) {
                    desiredActive = true
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                        context.startForegroundService(intent)
                    } else {
                        context.startService(intent)
                    }
                } else {
                    desiredActive = false
                    // Only call stopService once startForeground() has completed; stopping earlier triggers
                    // ForegroundServiceDidNotStartInTimeException on fast-finishing jobs.
                    if (isForeground) {
                        isForeground = false
                        context.stopService(intent)
                    }
                }
            }
        }
    }
}
