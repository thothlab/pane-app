package tech.thothlab.pane.helper

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.security.KeyChain
import android.util.Base64
import android.util.Log
import android.widget.Toast

/**
 * Receives a CA certificate from Pane (PEM, base64-encoded in an extra)
 * and hands it to the system KeyChain installer.
 *
 * The KeyChain.createInstallIntent() path is what we wanted from day
 * one: Android shows a single "Install this CA?" dialog plus a PIN
 * prompt, then the cert lands in the user trust store. No SAF picker,
 * no manual file pick — those only happen when the install is
 * initiated through CertInstaller's VIEW intent with a file URI, which
 * Android 11+ deliberately bounces through scoped storage.
 *
 * The dialog shown to the user is owned by the system, but the
 * "source app" attribution is THIS app (tech.thothlab.pane.helper),
 * not "shell" — so Samsung's One UI 8 "Shell can't install CAs"
 * block doesn't fire.
 *
 * Inputs (Intent extras):
 *   ca_pem_base64 — required, base64 of the PEM bytes
 *   ca_name       — optional, label shown in the install dialog
 *                   (default: "Pane Root CA")
 *
 * Launched by Pane via:
 *   adb shell am start
 *     -n tech.thothlab.pane.helper/.InstallCaActivity
 *     --es ca_pem_base64 <base64>
 *     --es ca_name "Pane Root CA"
 */
class InstallCaActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val base64 = intent.getStringExtra(EXTRA_PEM_B64)
        if (base64.isNullOrEmpty()) {
            Log.e(TAG, "no ca_pem_base64 extra; nothing to install")
            Toast.makeText(this, "Pane Helper: no certificate data", Toast.LENGTH_LONG).show()
            finish()
            return
        }

        val pemBytes = try {
            Base64.decode(base64, Base64.DEFAULT)
        } catch (e: IllegalArgumentException) {
            Log.e(TAG, "invalid base64 in ca_pem_base64", e)
            Toast.makeText(this, "Pane Helper: invalid certificate data", Toast.LENGTH_LONG).show()
            finish()
            return
        }

        val name = intent.getStringExtra(EXTRA_NAME) ?: "Pane Root CA"

        val installIntent = KeyChain.createInstallIntent().apply {
            // EXTRA_CERTIFICATE accepts the PEM bytes directly — the
            // platform parser handles both PEM and DER under the hood.
            putExtra(KeyChain.EXTRA_CERTIFICATE, pemBytes)
            putExtra(KeyChain.EXTRA_NAME, name)
        }

        try {
            startActivityForResult(installIntent, REQ_INSTALL)
        } catch (e: Exception) {
            Log.e(TAG, "failed to launch KeyChain install intent", e)
            Toast.makeText(this, "Pane Helper: ${e.message}", Toast.LENGTH_LONG).show()
            finish()
        }
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_INSTALL) {
            val ok = resultCode == RESULT_OK
            Log.i(TAG, "CA install result: ok=$ok resultCode=$resultCode")
            if (!ok) {
                // User cancelled or PIN attempts exhausted. We don't
                // re-try — Pane can re-launch us on the next Add device.
                Toast.makeText(
                    this,
                    "Pane CA install cancelled",
                    Toast.LENGTH_SHORT,
                ).show()
            }
            finish()
        }
    }

    companion object {
        private const val TAG = "PaneHelper"
        private const val REQ_INSTALL = 1
        private const val EXTRA_PEM_B64 = "ca_pem_base64"
        private const val EXTRA_NAME = "ca_name"
    }
}
