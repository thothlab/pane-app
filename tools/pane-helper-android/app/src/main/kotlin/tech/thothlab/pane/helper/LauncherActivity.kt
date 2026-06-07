package tech.thothlab.pane.helper

import android.app.Activity
import android.content.Intent
import android.os.Bundle

/**
 * Transparent launcher whose only job is to start the heartbeat service
 * and finish immediately.
 *
 * Earlier versions of this activity called `requestPermissions` for
 * POST_NOTIFICATIONS, then started the service from
 * `onRequestPermissionsResult`. That created a hard dependency on the
 * user tapping "Allow" on a system dialog before the watchdog wired
 * up — easy to miss during a USB-driven pairing flow, with no
 * recovery. Pane now grants POST_NOTIFICATIONS via `adb shell pm grant`
 * before launching this activity (same trick as WRITE_SECURE_SETTINGS),
 * so no dialog is needed.
 *
 * If the adb-side grant failed (very old Android, weird OEM policy),
 * the foreground service still runs — only the user-visible
 * notification is suppressed. Watchdog behavior is unaffected.
 *
 * Launched explicitly by Pane via
 * `adb shell am start -n tech.thothlab.pane.helper/.LauncherActivity`.
 */
class LauncherActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        startForegroundService(Intent(this, HeartbeatService::class.java))
        finish()
    }
}
