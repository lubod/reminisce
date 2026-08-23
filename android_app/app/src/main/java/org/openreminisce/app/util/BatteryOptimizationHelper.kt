package org.openreminisce.app.util

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import org.openreminisce.app.R

private const val TAG = "BatteryPrompt"

private val AGGRESSIVE_OEMS = listOf("HONOR", "HUAWEI", "XIAOMI", "OPPO", "VIVO", "REALME", "ONEPLUS")

/**
 * Honor/Huawei/Xiaomi builds silently freeze background uploads unless the
 * app is exempted from battery optimizations. Only prompt on those OEMs,
 * and ask a few times max.
 */
fun AppCompatActivity.maybePromptBatteryOptimization() {
    val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
    if (pm.isIgnoringBatteryOptimizations(packageName)) return

    val manufacturer = Build.MANUFACTURER.uppercase()
    if (!AGGRESSIVE_OEMS.any(manufacturer::startsWith)) return

    val prefs = getSharedPreferences("BackupState", Context.MODE_PRIVATE)
    val askedCount = prefs.getInt("battery_prompt_count", 0)
    if (askedCount >= 3) return // don't nag forever

    AlertDialog.Builder(this)
        .setTitle(getString(R.string.battery_prompt_title))
        .setMessage(getString(R.string.battery_prompt_message))
        .setPositiveButton(getString(R.string.battery_prompt_allow)) { _, _ ->
            try {
                startActivity(
                    Intent(
                        Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                        Uri.parse("package:$packageName")
                    )
                )
            } catch (e: Exception) {
                Log.w(TAG, "Battery optimization settings unavailable", e)
            }
        }
        .setNegativeButton(getString(R.string.battery_prompt_later), null)
        .show()

    prefs.edit().putInt("battery_prompt_count", askedCount + 1).apply()
}
