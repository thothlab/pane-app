package tech.thothlab.pane.helper

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.core.content.ContextCompat

/**
 * Transparent launcher whose only job is to request POST_NOTIFICATIONS
 * on first run (Android 13+) and then start the heartbeat service.
 *
 * Without the runtime permission grant, Android suppresses the FGS
 * notification — the service runs invisibly, the user can't see Pane
 * is intercepting their traffic, and there's no Stop button. That's a
 * support nightmare. One OS dialog tap fixes it forever.
 *
 * The activity finishes immediately after firing the service intent
 * so the user never sees a window. Pane launches us via
 * `adb shell am start -n tech.thothlab.pane.helper/.LauncherActivity`.
 */
class LauncherActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (needsNotificationPermission()) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQ_NOTIF)
        } else {
            startServiceAndExit()
        }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        // Result doesn't matter for our purposes — service starts either
        // way. If the user denied, the service runs invisibly; if they
        // accepted, the notification appears. We don't gate on it.
        startServiceAndExit()
    }

    private fun needsNotificationPermission(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return false
        return ContextCompat.checkSelfPermission(
            this, Manifest.permission.POST_NOTIFICATIONS,
        ) != PackageManager.PERMISSION_GRANTED
    }

    private fun startServiceAndExit() {
        startForegroundService(Intent(this, HeartbeatService::class.java))
        finish()
    }

    companion object {
        private const val REQ_NOTIF = 1
    }
}
