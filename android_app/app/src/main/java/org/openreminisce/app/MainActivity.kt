package org.openreminisce.app

import android.content.Intent
import android.os.Bundle
import android.util.Log
import android.view.Menu
import android.view.MenuItem
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.fragment.app.Fragment
import androidx.viewpager2.adapter.FragmentStateAdapter
import androidx.viewpager2.widget.ViewPager2
import androidx.work.WorkManager
import org.openreminisce.app.fragments.LocalMediaFragment
import org.openreminisce.app.fragments.RemoteMediaFragmentNative
import org.openreminisce.app.service.BackupController
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.DatabaseHelper
import org.openreminisce.app.util.LogCollector
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.util.SecureStorageHelper
import org.openreminisce.app.util.ThumbnailAuthInterceptor
import android.widget.TextView
import android.widget.Toast

class MainActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "MainActivity"
    }

    private lateinit var tabLocal: TextView
    private lateinit var tabRemote: TextView
    private lateinit var tabLocalUnderline: android.view.View
    private lateinit var tabRemoteUnderline: android.view.View
    private lateinit var viewPager: ViewPager2
    private lateinit var databaseHelper: DatabaseHelper
    private lateinit var backupController: BackupController

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        setContentView(R.layout.activity_main)

        setSupportActionBar(findViewById<androidx.appcompat.widget.Toolbar>(R.id.toolbar))
        supportActionBar?.title = getString(R.string.app_name)

        tabLocal = findViewById(R.id.tabLocal)
        tabRemote = findViewById(R.id.tabRemote)
        tabLocalUnderline = findViewById(R.id.tabLocalUnderline)
        tabRemoteUnderline = findViewById(R.id.tabRemoteUnderline)
        viewPager = findViewById(R.id.viewPager)

        databaseHelper = DatabaseHelper(this)
        backupController = BackupController(this, databaseHelper)
        backupController.attach()

        setupViewPager()

        // Ensure Glide is configured with the authenticated OkHttpClient
        (application as? ReminisceApplication)?.registerAuthClientWithGlide()

        viewPager.registerOnPageChangeCallback(object : ViewPager2.OnPageChangeCallback() {
            override fun onPageSelected(position: Int) {
                invalidateOptionsMenu()
            }
        })

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

        // Simple button-style tabs (no Material TabLayout). Selected tab is
        // highlighted by a white underline + full-white text.
        val selectTab: (Int) -> Unit = { pos ->
            val localActive = pos == 0
            val remoteActive = pos == 1
            tabLocal.setTextColor(if (localActive) 0xFFFFFFFF.toInt() else 0xAAFFFFFF.toInt())
            tabRemote.setTextColor(if (remoteActive) 0xFFFFFFFF.toInt() else 0xAAFFFFFF.toInt())
            tabLocalUnderline.visibility = if (localActive) android.view.View.VISIBLE else android.view.View.INVISIBLE
            tabRemoteUnderline.visibility = if (remoteActive) android.view.View.VISIBLE else android.view.View.INVISIBLE
        }
        findViewById<android.view.View>(R.id.tabLocalContainer).setOnClickListener { viewPager.currentItem = 0 }
        findViewById<android.view.View>(R.id.tabRemoteContainer).setOnClickListener { viewPager.currentItem = 1 }
        viewPager.registerOnPageChangeCallback(object : ViewPager2.OnPageChangeCallback() {
            override fun onPageSelected(position: Int) {
                selectTab(position)
            }
        })
        selectTab(0)
        Log.d(TAG, "Tabs header active: local='${tabLocal.text}' remote='${tabRemote.text}'")
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

    private fun startQuickBackup() = backupController.startQuickBackup()

    private fun startFullBackup() = backupController.startFullBackup()

    private fun stopBackup() = backupController.stopBackup()

    override fun onCreateOptionsMenu(menu: Menu): Boolean {
        menuInflater.inflate(R.menu.main_menu, menu)

        // Remove the toggle gallery item as it is replaced by tabs
        menu.findItem(R.id.action_toggle_gallery)?.isVisible = false

        return true
    }

    override fun onPrepareOptionsMenu(menu: Menu): Boolean {
        menu.findItem(R.id.action_stop_backup)?.isVisible = backupController.isBackingUp
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
        val baseUrl = PreferenceHelper.getServerUrl(this)
        val refreshToken = SecureStorageHelper.getRefreshToken(this)

        // Cancel running unique backup work without nuking WorkManager database globally
        try {
            WorkManager.getInstance(this).cancelUniqueWork(BackupController.UNIQUE_BACKUP_WORK_NAME)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to cancel work during logout", e)
        }

        getSharedPreferences("BackupState", MODE_PRIVATE)
            .edit()
            .remove("pending_work_id")
            .remove("backup_running_until")
            .apply()

        // Clear all local credentials synchronously before navigating to LoginActivity,
        // and revoke the refresh token on the server asynchronously.
        SecureStorageHelper.clearCredentials(this)
        ThumbnailAuthInterceptor.clearCachedToken()

        if (!refreshToken.isNullOrEmpty() && baseUrl.isNotEmpty()) {
            val appCtx = applicationContext
            Thread {
                AuthHelper.revokeRefreshTokenOnServer(appCtx, baseUrl, refreshToken)
            }.start()
        }

        // Navigate to LoginActivity
        val intent = Intent(this, LoginActivity::class.java)
        intent.flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK
        startActivity(intent)
        finish()
    }

    override fun onDestroy() {
        backupController.onDestroy()
        super.onDestroy()
    }
}
