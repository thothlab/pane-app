package tech.thothlab.pane.helper

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import java.io.BufferedReader
import java.io.IOException
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.net.InetSocketAddress
import java.net.Socket

/**
 * Foreground service that keeps a heartbeat socket open to Pane on the
 * laptop (via adb-reverse-forwarded 127.0.0.1:8890). When the socket
 * dies — because the user unplugged USB, killed Pane, or closed the
 * proxy — this service clears the device's `http_proxy` global setting
 * so the device stops sending traffic into a dead 127.0.0.1:8888 and
 * regains internet.
 *
 * Restore on reconnect is NOT this service's job — Pane's own
 * device_watchdog (Rust tokio task) re-applies http_proxy when it sees
 * the device come back. Splitting responsibilities this way keeps the
 * APK trivial and avoids two writers racing on Settings.Global.
 *
 * The proxy is only cleared if its current value matches what Pane
 * sets ("127.0.0.1:8888"). If the user has their own proxy configured,
 * we leave it alone.
 */
class HeartbeatService : Service() {

    @Volatile private var running = false
    private var worker: Thread? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (running) return START_STICKY
        running = true
        startForeground(NOTIF_ID, buildNotification(connected = false))
        worker = Thread { runHeartbeat() }.apply {
            name = "pane-heartbeat"
            isDaemon = true
            start()
        }
        Log.i(TAG, "HeartbeatService started")
        return START_STICKY
    }

    override fun onDestroy() {
        Log.i(TAG, "HeartbeatService stopping")
        running = false
        worker?.interrupt()
        clearProxyIfOurs()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    /**
     * Connect → ping/pong loop → on failure, sleep + retry.
     *
     * Clearing http_proxy only fires on a connected→disconnected
     * transition (i.e. an actually-established session went away).
     * Otherwise: if Pane sets http_proxy at T=0 and adb-reverse
     * appears at T=1, our retry loop ticking between T=0 and T=1
     * would otherwise clear the proxy Pane just set and the device
     * would briefly lose internet on every start. `wasConnected`
     * gates that.
     */
    private fun runHeartbeat() {
        var wasConnected = false
        while (running) {
            try {
                Socket().use { sock ->
                    sock.connect(InetSocketAddress("127.0.0.1", PORT), CONNECT_TIMEOUT_MS)
                    sock.soTimeout = PONG_TIMEOUT_MS
                    wasConnected = true
                    Log.i(TAG, "heartbeat connected")
                    updateNotification(connected = true)

                    val reader = BufferedReader(InputStreamReader(sock.getInputStream()))
                    val writer = OutputStreamWriter(sock.getOutputStream())

                    while (running) {
                        writer.write("PING\n")
                        writer.flush()
                        val line = reader.readLine() ?: throw IOException("EOF on read")
                        if (line != "PONG") throw IOException("bad reply: $line")
                        Thread.sleep(PING_INTERVAL_MS)
                    }
                }
            } catch (_: InterruptedException) {
                return
            } catch (e: IOException) {
                Log.i(TAG, "heartbeat dropped: ${e.message}")
                if (wasConnected) {
                    // Only clear on transition from established session
                    // to broken — see method doc above.
                    clearProxyIfOurs()
                    updateNotification(connected = false)
                    wasConnected = false
                }
                try { Thread.sleep(RECONNECT_INTERVAL_MS) } catch (_: InterruptedException) { return }
            } catch (e: Exception) {
                Log.w(TAG, "unexpected error in heartbeat loop", e)
                try { Thread.sleep(RECONNECT_INTERVAL_MS) } catch (_: InterruptedException) { return }
            }
        }
    }

    /**
     * Only touch http_proxy when its current value is exactly what Pane
     * set. Avoids stomping on a user's own proxy config if they had one
     * and we somehow ended up running anyway.
     */
    private fun clearProxyIfOurs() {
        try {
            val current = Settings.Global.getString(contentResolver, PROXY_KEY)
            if (current == OUR_PROXY) {
                Settings.Global.putString(contentResolver, PROXY_KEY, EMPTY_PROXY)
                Log.i(TAG, "cleared http_proxy (was $current)")
            } else {
                Log.d(TAG, "http_proxy is '$current', not ours, leaving alone")
            }
        } catch (e: SecurityException) {
            // WRITE_SECURE_SETTINGS was not granted via pm grant. The
            // service is now useless but harmless — log and carry on so
            // we don't crash-loop in onDestroy.
            Log.e(TAG, "WRITE_SECURE_SETTINGS not granted: ${e.message}")
        }
    }

    private fun updateNotification(connected: Boolean) {
        val nm = getSystemService(NotificationManager::class.java)
        nm.notify(NOTIF_ID, buildNotification(connected))
    }

    private fun buildNotification(connected: Boolean): Notification {
        val (titleRes, textRes) = if (connected) {
            R.string.notif_title_connected to R.string.notif_text_connected
        } else {
            R.string.notif_title_disconnected to R.string.notif_text_disconnected
        }
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_lock_lock)
            .setContentTitle(getString(titleRes))
            .setContentText(getString(textRes))
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
    }

    private fun createNotificationChannel() {
        val nm = getSystemService(NotificationManager::class.java)
        val ch = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.notif_channel_name),
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = getString(R.string.notif_channel_desc)
            setShowBadge(false)
        }
        nm.createNotificationChannel(ch)
    }

    companion object {
        private const val TAG = "PaneHelper"
        private const val CHANNEL_ID = "pane_connection"
        private const val NOTIF_ID = 1001

        // Heartbeat endpoint — Pane's TCP listener, adb-reverse-forwarded
        // from the device's 127.0.0.1:8890 to the laptop's 8890.
        private const val PORT = 8890

        // Use string literal instead of Settings.Global.HTTP_PROXY so we
        // don't trip the @Deprecated annotation on that field (still works,
        // just noisy). Same key on every Android version.
        private const val PROXY_KEY = "http_proxy"
        private const val OUR_PROXY = "127.0.0.1:8888"
        private const val EMPTY_PROXY = ":0" // Android idiom for "no proxy"

        private const val CONNECT_TIMEOUT_MS = 2_000
        private const val PONG_TIMEOUT_MS = 4_000
        private const val PING_INTERVAL_MS = 2_000L
        private const val RECONNECT_INTERVAL_MS = 5_000L
    }
}
