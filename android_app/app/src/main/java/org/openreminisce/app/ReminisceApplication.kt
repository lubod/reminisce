package org.openreminisce.app

import android.app.Application
import android.util.Log
import androidx.appcompat.app.AppCompatDelegate
import com.bumptech.glide.Glide
import com.bumptech.glide.integration.okhttp3.OkHttpUrlLoader
import com.bumptech.glide.load.model.GlideUrl
import okhttp3.Call
import org.openreminisce.app.util.AuthenticatedHttpClient
import org.openreminisce.app.util.LogCollector
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.util.SecureStorageHelper
import java.io.InputStream

class ReminisceApplication : Application() {
    companion object {
        private const val TAG = "ReminisceApplication"
    }

    override fun onCreate() {
        super.onCreate()

        // Persist fatal-crash stacks into the in-app log so "Share Logs"
        // contains them — otherwise they only live in system logcat.
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, throwable ->
            try {
                org.openreminisce.app.util.LogCollector.e(
                    "FATAL",
                    "Uncaught exception on thread ${thread.name}: ${throwable.javaClass.name}: ${throwable.message}"
                )
                for (el in throwable.stackTrace) {
                    org.openreminisce.app.util.LogCollector.e(
                        "FATAL",
                        "    at ${el.className}.${el.methodName}(${el.fileName}:${el.lineNumber})"
                    )
                }
                throwable.cause?.let { cause ->
                    org.openreminisce.app.util.LogCollector.e("FATAL", "Caused by: ${cause.javaClass.name}: ${cause.message}")
                }
            } catch (_: Throwable) {
                // never let the handler itself crash
            }
            previous?.uncaughtException(thread, throwable)
        }

        // Apply persisted theme preference (dark mode by default)
        val isDarkTheme = SecureStorageHelper.getThemePreference(this)
        AppCompatDelegate.setDefaultNightMode(
            if (isDarkTheme) AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO
        )

        // Migrate old preferences if needed
        migratePreferences()

        // Clear any legacy running-window hint left by older versions while preserving WorkManager state.
        getSharedPreferences("BackupState", MODE_PRIVATE).edit()
            .remove("backup_running_until")
            .apply()

        // Register the dynamic OkHttpClient with Glide
        registerAuthClientWithGlide()
    }

    private fun migratePreferences() {
        val prefs = getSharedPreferences("my_backup_prefs", MODE_PRIVATE)

        // Check if already migrated to relay mode
        if (prefs.getBoolean("migrated_to_relay_mode", false)) {
            return
        }

        Log.d(TAG, "Migrating preferences to relay mode...")

        if (!PreferenceHelper.isConfigured(this)) {
            // Not configured yet — clear credentials to force reconfiguration
            Log.d(TAG, "Not configured - clearing credentials to force reconfiguration")
            SecureStorageHelper.clearCredentials(this)
        }

        // Mark migration complete
        prefs.edit().putBoolean("migrated_to_relay_mode", true).apply()
        Log.d(TAG, "Migration to relay mode completed")
    }

    fun registerAuthClientWithGlide() {
        try {
            val callFactory = Call.Factory { request ->
                AuthenticatedHttpClient.getClient(this).newCall(request)
            }

            // Replace Glide's default URL loader with one dynamically delegating to our authenticated OkHttpClient
            Glide.get(this).registry.replace(
                GlideUrl::class.java,
                InputStream::class.java,
                OkHttpUrlLoader.Factory(callFactory)
            )

            LogCollector.d(TAG, "Registered dynamic OkHttpClient with Glide")
        } catch (e: Exception) {
            LogCollector.e(TAG, "Failed to register dynamic OkHttpClient with Glide", e)
        }
    }
}
