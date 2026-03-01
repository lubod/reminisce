package org.openreminisce.app

import android.app.Application
import android.util.Log
import androidx.appcompat.app.AppCompatDelegate
import androidx.work.Configuration
import androidx.work.WorkManager
import com.bumptech.glide.Glide
import com.bumptech.glide.integration.okhttp3.OkHttpUrlLoader
import com.bumptech.glide.load.model.GlideUrl
import org.openreminisce.app.util.AuthenticatedHttpClient
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.util.SecureStorageHelper
import java.io.InputStream

class ReminisceApplication : Application() {
    companion object {
        private const val TAG = "ReminisceApplication"
    }

    override fun onCreate() {
        super.onCreate()

        // Set dark theme as default
        AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_YES)

        // Migrate old preferences if needed
        migratePreferences()

        // Cancel all backup work BEFORE initializing WorkManager
        // This prevents WorkManager from auto-restarting interrupted work
        cleanupBackupWork()

        // Now manually initialize WorkManager
        initializeWorkManager()

        // Register the custom OkHttpClient with Glide
        registerAuthClientWithGlide()
    }

    private fun cleanupBackupWork() {
        try {
            Log.d("ReminisceApplication", "Starting cleanup of backup work...")

            // Initialize WorkManager with a minimal configuration just to clean up
            val config = Configuration.Builder()
                .setMinimumLoggingLevel(Log.DEBUG)
                .build()
            WorkManager.initialize(this, config)
            Log.d("ReminisceApplication", "WorkManager initialized for cleanup")

            val workManager = WorkManager.getInstance(this)

            // Cancel ALL work (not just by tag) to ensure nothing auto-restarts
            workManager.cancelAllWork()
            Log.d("ReminisceApplication", "Cancelled ALL work on app start")

            // Prune finished work
            workManager.pruneWork()
            Log.d("ReminisceApplication", "Pruned all finished work on app start")

            Log.d("ReminisceApplication", "Cleanup completed successfully")
        } catch (e: Exception) {
            Log.e("ReminisceApplication", "Error cleaning up backup work", e)
        }
    }

    private fun initializeWorkManager() {
        // WorkManager is already initialized in cleanupBackupWork()
        Log.d("ReminisceApplication", "WorkManager initialized")
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

    private fun registerAuthClientWithGlide() {
        if (!PreferenceHelper.isConfigured(this)) {
            Log.d("ReminisceApplication", "Not configured yet, skipping Glide auth client registration")
            return
        }

        val okHttpClient = AuthenticatedHttpClient.getClient(this)

        // Replace Glide's default URL loader with one using our authenticated client
        Glide.get(this).registry.replace(
            GlideUrl::class.java,
            InputStream::class.java,
            OkHttpUrlLoader.Factory(okHttpClient)
        )

        Log.d("ReminisceApplication", "Registered authenticated OkHttpClient with Glide")
    }
}
