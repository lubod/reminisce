package org.openreminisce.app

import android.content.Intent
import android.os.Bundle
import android.util.Log
import android.view.Menu
import android.view.MenuItem
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.fragment.app.Fragment
import androidx.viewpager2.adapter.FragmentStateAdapter
import androidx.viewpager2.widget.ViewPager2
import androidx.work.WorkManager
import org.openreminisce.app.fragments.LocalMediaFragment
import org.openreminisce.app.fragments.RemoteMediaFragmentNative
import org.openreminisce.app.service.BackupService
import org.openreminisce.app.util.BackupProgressDialogHelper
import org.openreminisce.app.util.DatabaseHelper
import org.openreminisce.app.util.LogCollector
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.util.SecureStorageHelper
import android.widget.Toast
import com.google.android.material.tabs.TabLayout
import com.google.android.material.tabs.TabLayoutMediator
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class MainActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "MainActivity"
    }

    private lateinit var tabLayout: TabLayout
    private lateinit var viewPager: ViewPager2
    private lateinit var databaseHelper: DatabaseHelper
    private lateinit var progressDialogHelper: BackupProgressDialogHelper
    private var isBackingUp = false
    private var currentBackupType = "image" // Track current backup type

    private val backupStatusReceiver = object : android.content.BroadcastReceiver() {
        override fun onReceive(context: android.content.Context?, intent: android.content.Intent?) {
            Log.d(TAG, "Received broadcast: ${intent?.action}")
            if (intent?.action == "org.openreminisce.app.BACKUP_STATUS") {
                val status = intent.getStringExtra("status")

                if (status == "completed" || status == "failed" || status == "cancelled") {
                    isBackingUp = false
                }

                if (status == "completed") {
                    handleBackupCompleted(intent)
                } else if (status == "failed" || status == "cancelled") {
                    handleBackupFailedOrCancelled(status)
                }
            } else if (intent?.action == "org.openreminisce.app.BACKUP_PROGRESS") {
                handleBackupProgress(intent)
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        
        setContentView(R.layout.activity_main)

        supportActionBar?.title = "Reminisce"

        tabLayout = findViewById(R.id.tabLayout)
        viewPager = findViewById(R.id.viewPager)

        databaseHelper = DatabaseHelper(this)
        progressDialogHelper = BackupProgressDialogHelper(this)

        setupViewPager()

        viewPager.registerOnPageChangeCallback(object : ViewPager2.OnPageChangeCallback() {
            override fun onPageSelected(position: Int) {
                invalidateOptionsMenu()
            }
        })

        // Register broadcast receiver for backup status and progress updates
        val filter = android.content.IntentFilter("org.openreminisce.app.BACKUP_STATUS")
        filter.addAction("org.openreminisce.app.BACKUP_PROGRESS")

        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(backupStatusReceiver, filter, android.content.Context.RECEIVER_NOT_EXPORTED)
        } else {
            androidx.core.content.ContextCompat.registerReceiver(this, backupStatusReceiver, filter, androidx.core.content.ContextCompat.RECEIVER_NOT_EXPORTED)
        }

        // Handle back button press to handle WebView navigation if in remote tab?
        // Actually navigation is handled inside fragments or separate activities if pushed.
        // For ViewPager, maybe back should go to first tab? Or standard back behavior.
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (viewPager.currentItem != 0) {
                    viewPager.currentItem = 0
                } else {
                    finishAffinity()
                }
            }
        })
    }

    private fun setupViewPager() {
        val adapter = ViewPagerAdapter(this)
        viewPager.adapter = adapter
        viewPager.isUserInputEnabled = false

        TabLayoutMediator(tabLayout, viewPager) { tab, position ->
            when (position) {
                0 -> tab.text = getString(R.string.local_gallery)
                1 -> tab.text = getString(R.string.server_gallery)
            }
        }.attach()
    }

    private inner class ViewPagerAdapter(activity: AppCompatActivity) : FragmentStateAdapter(activity) {
        override fun getItemCount(): Int = 2

        override fun createFragment(position: Int): Fragment {
            return when (position) {
                0 -> LocalMediaFragment()
                1 -> RemoteMediaFragmentNative()
                else -> throw IllegalStateException("Invalid position")
            }
        }
    }

    private fun startQuickBackup() {
        if (isBackingUp) {
            return
        }

        isBackingUp = true

        // Clear any previous cancel flag
        val prefs = getSharedPreferences("BackupState", MODE_PRIVATE)
        prefs.edit().putBoolean("cancel_backup", false).apply()

        // Get the last backup timestamp based on current backup type
        val lastBackupTimestamp = if (currentBackupType == "video") {
            databaseHelper.getLastVideoBackupTimestamp()
        } else {
            databaseHelper.getLastImageBackupTimestamp()
        }

        val timestampText = if (lastBackupTimestamp != null && lastBackupTimestamp > 0) {
            val date = Date(lastBackupTimestamp * 1000)
            val dateFormat = SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.getDefault())
            "since last full upload " + dateFormat.format(date)
        } else {
            "since the beginning"
        }

        // Show progress dialog
        progressDialogHelper.show(
            getString(R.string.starting_backup),
            isQuickBackup = true,
            quickBackupTimestampText = timestampText
        ) {
            stopBackup()
        }

        // Start backup service with intent
        val intent = Intent(this, BackupService::class.java).apply {
            putExtra("backup_type", currentBackupType)
            putExtra("quick_backup", true)
        }
        startService(intent)
    }

    private fun startFullBackup() {
        if (isBackingUp) {
            return
        }

        isBackingUp = true

        // Clear any previous cancel flag
        val prefs = getSharedPreferences("BackupState", MODE_PRIVATE)
        prefs.edit().putBoolean("cancel_backup", false).apply()

        // Show progress dialog
        progressDialogHelper.show(
            "Full upload...",
            isQuickBackup = false
        ) {
            stopBackup()
        }

        // Start backup service with intent
        val intent = Intent(this, BackupService::class.java).apply {
            putExtra("backup_type", currentBackupType)
            putExtra("quick_backup", false)
        }
        startService(intent)
    }

    private fun stopBackup() {
        Log.d(TAG, "stopBackup() called")

        // Set cancellation flag
        val prefs = getSharedPreferences("BackupState", MODE_PRIVATE)
        prefs.edit().apply {
            putBoolean("is_backup_running", false)
            putBoolean("cancel_backup", true)
            remove("backup_type")
            remove("is_quick_backup")
            apply()
        }

        val workManager = WorkManager.getInstance(this)
        workManager.cancelAllWork()

        val serviceIntent = Intent(this, BackupService::class.java)
        stopService(serviceIntent)

        isBackingUp = false
        progressDialogHelper.dismiss()
    }

    private fun handleBackupCompleted(intent: Intent) {
        val successfullyBackedUp = intent.getIntExtra("successfullyBackedUp", 0)
        val totalProcessed = intent.getIntExtra("totalProcessed", 0)
        val skippedExisting = intent.getIntExtra("skippedExisting", 0)
        val failedCount = intent.getIntExtra("failedCount", 0)
        val type = intent.getStringExtra("type") ?: "full"

        runOnUiThread {
            progressDialogHelper.showCompletion(
                successfullyBackedUp = successfullyBackedUp,
                totalProcessed = totalProcessed,
                skippedExisting = skippedExisting,
                failedCount = failedCount,
                backupType = currentBackupType,
                isQuickBackup = type == "quick"
            ) {
                // No specific action on dismiss needed for general MainActivity context
            }
        }
    }

    private fun handleBackupFailedOrCancelled(status: String?) {
        runOnUiThread {
            progressDialogHelper.showFailure(status ?: "failed") {
                // Nothing special needed on dismiss
            }
        }
    }

    private fun handleBackupProgress(intent: Intent) {
        val overallProgress = intent.getFloatExtra("overallProgress", 0f)
        val currentAction = intent.getStringExtra("currentAction") ?: "unknown"
        val currentFile = intent.getStringExtra("currentFile") ?: "unknown"
        val fileIndex = intent.getIntExtra("fileIndex", 0)
        val totalFiles = intent.getIntExtra("totalFiles", 0)
        val backedUpCount = intent.getIntExtra("backedUpCount", 0)
        val skippedCount = intent.getIntExtra("skippedCount", 0)
        val failedCount = intent.getIntExtra("failedCount", 0)
        val fileProgress = intent.getFloatExtra("fileProgress", 0f)
        val fileUploadProgress = intent.getFloatExtra("fileUploadProgress", 0f)

        runOnUiThread {
            progressDialogHelper.updateProgress(
                overallProgress = overallProgress,
                currentAction = currentAction,
                currentFile = currentFile,
                fileIndex = fileIndex,
                totalFiles = totalFiles,
                backedUpCount = backedUpCount,
                skippedCount = skippedCount,
                failedCount = failedCount,
                fileProgress = fileProgress,
                fileUploadProgress = fileUploadProgress
            )
        }
    }

    override fun onCreateOptionsMenu(menu: Menu): Boolean {
        menuInflater.inflate(R.menu.main_menu, menu)
        
        // Remove the toggle gallery item as it is replaced by tabs
        menu.findItem(R.id.action_toggle_gallery)?.isVisible = false
        
        return true
    }

    override fun onPrepareOptionsMenu(menu: Menu): Boolean {
        menu.findItem(R.id.action_stop_backup)?.isVisible = isBackingUp
        return super.onPrepareOptionsMenu(menu)
    }

    override fun onOptionsItemSelected(item: MenuItem): Boolean {
        return when (item.itemId) {
            R.id.action_quick_backup -> {
                startQuickBackup()
                true
            }
            R.id.action_backup_all -> {
                startFullBackup()
                true
            }
            R.id.action_stop_backup -> {
                stopBackup()
                true
            }
            R.id.action_logout -> {
                logout()
                true
            }
            R.id.action_toggle_theme -> {
                toggleTheme()
                true
            }
            R.id.action_share_logs -> {
                shareLogs()
                true
            }
            R.id.action_clear_hash_cache -> {
                clearHashCache()
                true
            }
            else -> super.onOptionsItemSelected(item)
        }
    }

    private fun clearHashCache() {
        databaseHelper.clearAllHashes()
        Toast.makeText(this, getString(R.string.hash_cache_cleared), Toast.LENGTH_SHORT).show()
    }

    private fun toggleTheme() {
        val currentNightMode = resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK
        val newMode = if (currentNightMode == android.content.res.Configuration.UI_MODE_NIGHT_YES) {
            androidx.appcompat.app.AppCompatDelegate.MODE_NIGHT_NO
        } else {
            androidx.appcompat.app.AppCompatDelegate.MODE_NIGHT_YES
        }

        androidx.appcompat.app.AppCompatDelegate.setDefaultNightMode(newMode)
        SecureStorageHelper.setThemePreference(this, newMode == androidx.appcompat.app.AppCompatDelegate.MODE_NIGHT_YES)
    }

    private fun shareLogs() {
        // Fetch fresh Rust logs first
        LogCollector.fetchRustLogs()

        val logs = LogCollector.getLogs()
        if (logs.isEmpty()) {
            Toast.makeText(this, getString(R.string.no_logs_to_share), Toast.LENGTH_SHORT).show()
            return
        }

        // Add device and app info header
        val deviceInfo = buildString {
            appendLine("=== Reminisce Debug Logs ===")
            appendLine("Timestamp: ${java.text.SimpleDateFormat("yyyy-MM-dd HH:mm:ss", java.util.Locale.US).format(java.util.Date())}")
            appendLine("Device: ${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL}")
            appendLine("Android: ${android.os.Build.VERSION.RELEASE} (API ${android.os.Build.VERSION.SDK_INT})")
            appendLine("App Version: ${packageManager.getPackageInfo(packageName, 0).versionName}")
            appendLine("Server: ${PreferenceHelper.getServerUrl(this@MainActivity)}")
            appendLine("=============================")
            appendLine()
        }

        val fullLogs = deviceInfo + logs

        // Create share intent
        val shareIntent = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_SUBJECT, "Reminisce Debug Logs")
            putExtra(Intent.EXTRA_TEXT, fullLogs)
        }

        startActivity(Intent.createChooser(shareIntent, "Share logs via"))
    }

    private fun logout() {
        // Clear all credentials and tokens
        SecureStorageHelper.clearCredentials(this)

        // Navigate to LoginActivity
        val intent = Intent(this, LoginActivity::class.java)
        intent.flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK
        startActivity(intent)
        finish()
    }

    override fun onDestroy() {
        super.onDestroy()
        try {
            unregisterReceiver(backupStatusReceiver)
        } catch (e: Exception) {
            // Receiver was not registered or already unregistered
        }
        progressDialogHelper.dismiss()
    }
}
