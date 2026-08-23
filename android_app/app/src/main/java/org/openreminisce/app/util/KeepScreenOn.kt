package org.openreminisce.app.util

import android.view.WindowManager
import androidx.appcompat.app.AppCompatActivity

/**
 * Keeps the screen on while a backup is running and the app is visible.
 *
 * The backup itself already runs behind a PARTIAL wake lock plus a dataSync
 * foreground service, but on aggressive OEM builds (Honor/Huawei in
 * particular) uploads still stall when the display dozes off. While the user
 * is watching progress we simply hold FLAG_KEEP_SCREEN_ON so the device never
 * sleeps mid-upload; when the app is not visible nothing is held.
 */
fun AppCompatActivity.applyKeepScreenOn(active: Boolean) {
    if (active) {
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    } else {
        window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    }
}
