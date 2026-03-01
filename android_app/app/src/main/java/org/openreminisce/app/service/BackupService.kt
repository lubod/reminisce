package org.openreminisce.app.service

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.work.*
import org.openreminisce.app.R
import java.util.concurrent.TimeUnit

class BackupService : Service() {
    companion object {
        private const val TAG = "BackupService"
        private const val NOTIFICATION_ID = 1
        private const val CHANNEL_ID = "backup_channel"
        private const val PREFS_NAME = "BackupState"
        private const val KEY_IS_BACKUP_RUNNING = "is_backup_running"
        private const val KEY_BACKUP_TYPE = "backup_type"
        private const val KEY_IS_QUICK_BACKUP = "is_quick_backup"
    }

    override fun onBind(intent: Intent?): IBinder? {
        return null
    }
    
    private fun setBackupRunning(isRunning: Boolean, backupType: String = "image", isQuickBackup: Boolean = false) {
        val prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
        with(prefs.edit()) {
            putBoolean(KEY_IS_BACKUP_RUNNING, isRunning)
            if (isRunning) {
                putString(KEY_BACKUP_TYPE, backupType)
                putBoolean(KEY_IS_QUICK_BACKUP, isQuickBackup)
            } else {
                // Remove backup type and quick backup flags when backup stops
                remove(KEY_BACKUP_TYPE)
                remove(KEY_IS_QUICK_BACKUP)
            }
            apply()
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.d(TAG, "Backup service started")
        Log.d(TAG, "Intent action: ${intent?.action}")
        Log.d(TAG, "Backup type from intent: ${intent?.getStringExtra("backup_type")}")
        Log.d(TAG, "Quick backup from intent: ${intent?.getBooleanExtra("quick_backup", false)}")
        
        // Set backup as running in persistent storage
        setBackupRunning(true, intent?.getStringExtra("backup_type") ?: "image", intent?.getBooleanExtra("quick_backup", false) ?: false)
        
        // Create notification channel for Android 8.0 and above
        createNotificationChannel()
        
        // Create a notification for the foreground service with HIGH priority to prevent sleep
        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Upload Service")
            .setContentText("Performing upload operation...")
            .setSmallIcon(R.drawable.ic_launcher_foreground) // Use app's icon instead of generic one
            .setPriority(NotificationCompat.PRIORITY_HIGH) // HIGH priority to prevent system from killing it
            .setOngoing(true) // Make it ongoing to indicate it's a foreground service
            .setCategory(NotificationCompat.CATEGORY_SERVICE) // Set proper category
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC) // Make notification visible on lock screen
            .build()
        
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        
        // Start the actual backup work using WorkManager
        startBackupWork(intent)
        
        return START_STICKY
    }
    
    private fun createNotificationChannel() {
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
            val channel = android.app.NotificationChannel(
                CHANNEL_ID,
                "Upload Service Channel",
                android.app.NotificationManager.IMPORTANCE_HIGH // HIGH importance to prevent service from sleeping
            ).apply {
                description = "Notifications for upload service - keeps upload running with screen off"
                setShowBadge(true)
                lockscreenVisibility = android.app.Notification.VISIBILITY_PUBLIC
            }

            val notificationManager = getSystemService(android.app.NotificationManager::class.java)
            notificationManager.createNotificationChannel(channel)
        }
    }
    
    private var currentWorkId: java.util.UUID? = null
    private var workInfoObserver: androidx.lifecycle.Observer<androidx.work.WorkInfo>? = null
    private val handler = android.os.Handler(android.os.Looper.getMainLooper())
    private val pollingRunnable: Runnable = object : Runnable {
        override fun run() {
            // Periodically check work status as a backup mechanism
            currentWorkId?.let { workId ->
                // Use a background thread for this operation
                Thread {
                    try {
                        val workInfo = WorkManager.getInstance(this@BackupService).getWorkInfoById(workId).get()
                        if (workInfo != null && workInfo.state.isFinished) {
                            Log.d(TAG, "Polling detected finished work: ${workInfo.state}")
                            handleWorkCompletion(workInfo)
                        } else {
                            // Continue polling if not finished
                            handler.postDelayed(this, 5000) // Check every 5 seconds
                        }
                    } catch (e: Exception) {
                        Log.e(TAG, "Error polling work status", e)
                        handler.postDelayed(this, 5000) // Retry on error
                    }
                }.start()
            }
        }
    }

    private fun startBackupWork(intent: Intent?) {
        val backupType = intent?.getStringExtra("backup_type") ?: "image"
        val quickBackup = intent?.getBooleanExtra("quick_backup", false) ?: false

        Log.d(TAG, "Starting backup work - Type: $backupType, Quick: $quickBackup")

        val constraints = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.NOT_REQUIRED)  // Remove network constraint since backup is working
            .build()

        val inputData = Data.Builder()
            .putString("backup_type", backupType)
            .putBoolean("quick_backup", quickBackup)
            .build()

        val backupWorkRequest = OneTimeWorkRequestBuilder<BackupWorker>()
            .setConstraints(constraints)
            .setInputData(inputData)
            .addTag("backup_work")  // Add tag for tracking
            .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST) // Keep wake lock during execution
            .build()

        // Store the work ID so we can cancel it later
        currentWorkId = backupWorkRequest.id

        Log.d(TAG, "Enqueuing backup work request with ID: ${backupWorkRequest.id}")
        WorkManager.getInstance(this).enqueue(backupWorkRequest)

        // Remove any existing observer to prevent memory leaks
        workInfoObserver?.let { observer ->
            WorkManager.getInstance(this).getWorkInfoByIdLiveData(backupWorkRequest.id)
                .removeObserver(observer)
            Log.d(TAG, "Removed previous work info observer")
        }

        // Listen for work completion and progress updates
        val observer = androidx.lifecycle.Observer<androidx.work.WorkInfo> { workInfo ->
                Log.d(TAG, "Work info updated: ${workInfo.state}")
                Log.d(TAG, "Work ID: ${backupWorkRequest.id}")
                Log.d(TAG, "Work tags: ${workInfo.tags.joinToString(", ")}")
                Log.d(TAG, "Work run attempt count: ${workInfo.runAttemptCount}")
                Log.d(TAG, "Work output data: ${workInfo.outputData}")

                // Handle work completion
                if (workInfo.state.isFinished) {
                    handleWorkCompletion(workInfo, backupType, quickBackup)
                } else {
                    // Only handle progress updates when not finished
                    // Handle progress updates
                    workInfo.progress.let { progress ->
                        val overallProgress = progress.getFloat("overallProgress", 0f)
                        Log.d(TAG, "Sending progress: ${overallProgress * 100}%") // Debug log
                        val progressIntent = android.content.Intent("org.openreminisce.app.BACKUP_PROGRESS")
                        progressIntent.putExtra("overallProgress", overallProgress)
                        progressIntent.putExtra("currentAction", progress.getString("currentAction"))
                        progressIntent.putExtra("currentFile", progress.getString("currentFile"))
                        progressIntent.putExtra("fileIndex", progress.getInt("fileIndex", 0))
                        progressIntent.putExtra("totalFiles", progress.getInt("totalFiles", 0))
                        progressIntent.putExtra("backedUpCount", progress.getInt("backedUpCount", 0))
                        progressIntent.putExtra("skippedCount", progress.getInt("skippedCount", 0))
                        progressIntent.putExtra("failedCount", progress.getInt("failedCount", 0))
                        progressIntent.putExtra("fileProgress", progress.getFloat("fileProgress", 0f))
                        progressIntent.putExtra("fileUploadProgress", progress.getFloat("fileUploadProgress", 0f))
                        progressIntent.setPackage(this.packageName) // Restrict broadcast to this app only

                        this.sendBroadcast(progressIntent)
                    }

                    Log.d(TAG, "Work is not finished, current state: ${workInfo.state}, scheduled for execution when constraints are met")
                }
            }

        // Store the observer reference
        workInfoObserver = observer

        // Attach the observer to LiveData
        WorkManager.getInstance(this).getWorkInfoByIdLiveData(backupWorkRequest.id)
            .observeForever(observer)
        Log.d(TAG, "Attached work info observer")

        // Start polling as a backup mechanism in case observeForever stops working
        handler.postDelayed(pollingRunnable, 10000) // Start polling after 10 seconds
    }

    private fun handleWorkCompletion(workInfo: WorkInfo, @Suppress("UNUSED_PARAMETER") backupType: String? = null, quickBackup: Boolean? = null) {
        Log.d(TAG, "Work is finished with state: ${workInfo.state}")

        // Get quick backup flag from shared preferences if not provided
        val prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
        val actualQuickBackup = quickBackup ?: prefs.getBoolean(KEY_IS_QUICK_BACKUP, false)

        val broadcastIntent = android.content.Intent("org.openreminisce.app.BACKUP_STATUS")

        // Determine the status based on the work state
        val status = when (workInfo.state) {
            androidx.work.WorkInfo.State.SUCCEEDED -> "completed"
            androidx.work.WorkInfo.State.FAILED -> "failed"
            androidx.work.WorkInfo.State.CANCELLED -> "cancelled"
            else -> "completed" // Default to completed for any other finished state
        }

        Log.d(TAG, "Sending completion status: $status")
        broadcastIntent.putExtra("status", status)
        broadcastIntent.putExtra("type", if (actualQuickBackup) "quick" else "full")
        broadcastIntent.setPackage(this.packageName) // Restrict broadcast to this app only

        // Add detailed backup results if available (only for successful work)
        if (workInfo.state == androidx.work.WorkInfo.State.SUCCEEDED) {
            val outputData = workInfo.outputData
            val successfullyBackedUp = outputData.getInt("successfullyBackedUp", 0)
            val totalProcessed = outputData.getInt("totalProcessed", 0)
            val skippedExisting = outputData.getInt("skippedExisting", 0)
            val failedCount = outputData.getInt("failedCount", 0)
            val failedFiles = outputData.getStringArray("failedFiles")

            broadcastIntent.putExtra("successfullyBackedUp", successfullyBackedUp)
            broadcastIntent.putExtra("totalProcessed", totalProcessed)
            broadcastIntent.putExtra("skippedExisting", skippedExisting)
            broadcastIntent.putExtra("failedCount", failedCount)
            if (failedFiles != null && failedFiles.isNotEmpty()) {
                broadcastIntent.putExtra("failedFiles", failedFiles)
            }

            Log.d(TAG, "Backup results - Success: $successfullyBackedUp, Processed: $totalProcessed, Skipped: $skippedExisting, Failed: $failedCount")
        }

        Log.d(TAG, "Sending broadcast: org.openreminisce.app.BACKUP_STATUS")
        this.sendBroadcast(broadcastIntent)
        Log.d(TAG, "Broadcast sent successfully")

        // Remove observer when work is finished to prevent memory leak
        workInfoObserver?.let { observer ->
            currentWorkId?.let { workId ->
                WorkManager.getInstance(this).getWorkInfoByIdLiveData(workId)
                    .removeObserver(observer)
                workInfoObserver = null
                Log.d(TAG, "Removed work info observer after completion")
            }
        }

        // Stop polling
        handler.removeCallbacks(pollingRunnable)

        Log.d(TAG, "Stopping backup service")
        // Clear the backup running state
        setBackupRunning(false)
        stopSelf()
    }

    override fun onDestroy() {
        super.onDestroy()
        // Clean up observer when service is destroyed
        workInfoObserver?.let { observer ->
            currentWorkId?.let { workId ->
                WorkManager.getInstance(this).getWorkInfoByIdLiveData(workId)
                    .removeObserver(observer)
                Log.d(TAG, "Removed work info observer in onDestroy")
            }
            workInfoObserver = null
        }
    }
}