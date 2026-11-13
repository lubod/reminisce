package org.openreminisce.app.service

import android.content.Context
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.Data
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.WorkInfo
import androidx.work.WorkManager
import org.openreminisce.app.R
import org.openreminisce.app.util.BackupProgressDialogHelper
import org.openreminisce.app.util.DatabaseHelper
import org.openreminisce.app.util.LogCollector
import org.openreminisce.app.util.applyKeepScreenOn
import org.openreminisce.app.util.maybePromptBatteryOptimization
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.UUID
import java.util.concurrent.TimeUnit

/**
 * Shared WorkManager-backed backup lifecycle coordinator used by MainActivity and WebGalleryActivity.
 * Eliminates the redundant foreground service + broadcast proxy while preserving progress UI across rotations.
 */
class BackupController(
    private val activity: AppCompatActivity,
    private val databaseHelper: DatabaseHelper,
    private val onBackingUpChanged: (Boolean) -> Unit = {}
) {
    companion object {
        private const val TAG = "BackupController"
        const val UNIQUE_BACKUP_WORK_NAME = "reminisce_backup_work"
        private const val PREFS_NAME = "BackupState"
        private const val KEY_CANCEL_BACKUP = "cancel_backup"
        private const val KEY_IS_QUICK_BACKUP = "is_quick_backup"
        private const val KEY_PENDING_WORK_ID = "pending_work_id"
        private const val KEY_LAST_HANDLED_WORK_ID = "last_handled_work_id"
    }

    private val progressDialogHelper = BackupProgressDialogHelper(activity)
    private var activeWorkId: UUID? = null

    var isBackingUp: Boolean = false
        private set

    fun attach() {
        WorkManager.getInstance(activity.applicationContext)
            .getWorkInfosForUniqueWorkLiveData(UNIQUE_BACKUP_WORK_NAME)
            .observe(activity) { workInfos ->
                val info = workInfos?.firstOrNull() ?: return@observe
                handleWorkInfo(info)
            }
    }

    fun startQuickBackup() {
        startBackup(quickBackup = true)
    }

    fun startFullBackup() {
        startBackup(quickBackup = false)
    }

    private fun startBackup(quickBackup: Boolean) {
        try {
            if (isBackingUp) {
                LogCollector.d(TAG, "Backup request ignored — backup already running")
                Toast.makeText(activity, "Upload already running", Toast.LENGTH_SHORT).show()
                return
            }

            activity.maybePromptBatteryOptimization()
            setBackingUp(true)

            val prefs = activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            prefs.edit()
                .putBoolean(KEY_CANCEL_BACKUP, false)
                .putBoolean(KEY_IS_QUICK_BACKUP, quickBackup)
                .apply()

            if (quickBackup) {
                progressDialogHelper.show(
                    activity.getString(R.string.starting_backup),
                    isQuickBackup = true,
                    quickBackupTimestampText = formatQuickBackupSinceText()
                ) {
                    stopBackup()
                }
            } else {
                progressDialogHelper.show(
                    "Full upload...",
                    isQuickBackup = false
                ) {
                    stopBackup()
                }
            }

            val constraints = Constraints.Builder()
                .setRequiredNetworkType(NetworkType.CONNECTED)
                .build()

            val inputData = Data.Builder()
                .putString("backup_type", "all")
                .putBoolean("quick_backup", quickBackup)
                .build()

            val request = OneTimeWorkRequestBuilder<BackupWorker>()
                .setConstraints(constraints)
                .setInputData(inputData)
                .addTag("backup_work")
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .build()

            activeWorkId = request.id
            prefs.edit().putString(KEY_PENDING_WORK_ID, request.id.toString()).apply()

            WorkManager.getInstance(activity.applicationContext).enqueueUniqueWork(
                UNIQUE_BACKUP_WORK_NAME,
                ExistingWorkPolicy.REPLACE,
                request
            )
            LogCollector.i(TAG, "Enqueued unique backup work ${request.id} (quick=$quickBackup)")
        } catch (t: Throwable) {
            LogCollector.e(TAG, "startBackup crashed: ${t.javaClass.name}: ${t.message}", t)
            Toast.makeText(activity, "Backup failed to start: ${t.message}", Toast.LENGTH_LONG).show()
            setBackingUp(false)
        }
    }

    fun stopBackup() {
        LogCollector.d(TAG, "stopBackup() called")
        val prefs = activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        prefs.edit()
            .putBoolean(KEY_CANCEL_BACKUP, true)
            .remove(KEY_PENDING_WORK_ID)
            .remove(KEY_IS_QUICK_BACKUP)
            .remove("backup_running_until")
            .apply()

        WorkManager.getInstance(activity.applicationContext).cancelUniqueWork(UNIQUE_BACKUP_WORK_NAME)
        activeWorkId = null
        setBackingUp(false)
        progressDialogHelper.dismiss()
    }

    fun onDestroy() {
        activity.applyKeepScreenOn(false)
        progressDialogHelper.dismiss()
    }

    private fun handleWorkInfo(info: WorkInfo) {
        val prefs = activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val isQuickBackup = prefs.getBoolean(KEY_IS_QUICK_BACKUP, false)

        if (!info.state.isFinished) {
            // Reattach UI if activity restarted while backup worker is running.
            if (prefs.getBoolean(KEY_CANCEL_BACKUP, false)) {
                return
            }
            activeWorkId = info.id
            setBackingUp(true)
            prefs.edit().putString(KEY_PENDING_WORK_ID, info.id.toString()).apply()

            if (!progressDialogHelper.isShowing() && !activity.isFinishing && !activity.isDestroyed) {
                progressDialogHelper.show(
                    if (isQuickBackup) activity.getString(R.string.starting_backup) else "Full upload...",
                    isQuickBackup = isQuickBackup,
                    quickBackupTimestampText = if (isQuickBackup) formatQuickBackupSinceText() else null
                ) {
                    stopBackup()
                }
            }

            val progress = info.progress
            val totalFiles = progress.getInt("totalFiles", 0)
            val currentAction = progress.getString("currentAction")
            if (totalFiles > 0 || !currentAction.isNullOrEmpty()) {
                progressDialogHelper.updateProgress(
                    overallProgress = progress.getFloat("overallProgress", 0f),
                    currentAction = currentAction ?: "working",
                    currentFile = progress.getString("currentFile") ?: "",
                    fileIndex = progress.getInt("fileIndex", 0),
                    totalFiles = totalFiles,
                    backedUpCount = progress.getInt("backedUpCount", 0),
                    skippedCount = progress.getInt("skippedCount", 0),
                    failedCount = progress.getInt("failedCount", 0),
                    fileProgress = progress.getFloat("fileProgress", 0f),
                    fileUploadProgress = progress.getFloat("fileUploadProgress", 0f)
                )
            }
            return
        }

        if (activeWorkId == info.id) {
            activeWorkId = null
        }
        setBackingUp(false)

        val workIdStr = info.id.toString()
        val pendingWorkId = prefs.getString(KEY_PENDING_WORK_ID, null)
        val lastHandledWorkId = prefs.getString(KEY_LAST_HANDLED_WORK_ID, null)

        if (workIdStr != pendingWorkId || workIdStr == lastHandledWorkId) {
            return
        }

        prefs.edit()
            .putString(KEY_LAST_HANDLED_WORK_ID, workIdStr)
            .remove(KEY_PENDING_WORK_ID)
            .apply()

        if (activity.isFinishing || activity.isDestroyed) {
            return
        }

        when (info.state) {
            WorkInfo.State.SUCCEEDED -> {
                val output = info.outputData
                progressDialogHelper.showCompletion(
                    successfullyBackedUp = output.getInt("successfullyBackedUp", 0),
                    totalProcessed = output.getInt("totalProcessed", 0),
                    skippedExisting = output.getInt("skippedExisting", 0),
                    failedCount = output.getInt("failedCount", 0),
                    backupType = "all",
                    isQuickBackup = isQuickBackup
                ) {}
            }
            WorkInfo.State.FAILED -> {
                progressDialogHelper.showFailure("failed") {}
            }
            WorkInfo.State.CANCELLED -> {
                progressDialogHelper.showFailure("cancelled") {}
            }
            else -> progressDialogHelper.dismiss()
        }
    }

    private fun setBackingUp(backingUp: Boolean) {
        if (isBackingUp != backingUp) {
            isBackingUp = backingUp
            activity.invalidateOptionsMenu()
            onBackingUpChanged(backingUp)
        }
        activity.applyKeepScreenOn(backingUp)
    }

    private fun formatQuickBackupSinceText(): String {
        val imageTs = databaseHelper.getLastImageBackupTimestamp()
        val videoTs = databaseHelper.getLastVideoBackupTimestamp()
        val lastBackupTimestamp = listOfNotNull(imageTs, videoTs).minOrNull()
        return if (lastBackupTimestamp != null && lastBackupTimestamp > 0) {
            val date = Date(lastBackupTimestamp * 1000)
            val dateFormat = SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.getDefault())
            "since last full upload " + dateFormat.format(date)
        } else {
            "since the beginning"
        }
    }
}
