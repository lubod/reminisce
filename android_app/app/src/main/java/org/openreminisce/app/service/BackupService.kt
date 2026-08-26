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
        private const val KEY_BACKUP_TYPE = "backup_type"
        private const val KEY_IS_QUICK_BACKUP = "is_quick_backup"
        private const val KEY_BACKUP_RUNNING_UNTIL = "backup_running_until"
        private const val BACKUP_RUNNING_WINDOW_MS = 15L * 60 * 1000
    }

    override fun onBind(intent: Intent?): IBinder? {
        return null
    }
    
    private fun setBackupRunning(isRunning: Boolean, backupType: String = "image", isQuickBackup: Boolean = false) {
        val prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
        with(prefs.edit()) {
            if (isRunning) {
                putString(KEY_BACKUP_TYPE, backupType)
                putBoolean(KEY_IS_QUICK_BACKUP, isQuickBackup)
            } else {
                // Remove backup type and quick backup flags when backup stops
                remove(KEY_BACKUP_TYPE)
                remove(KEY_IS_QUICK_BACKUP)
                remove(KEY_BACKUP_RUNNING_UNTIL)
            }
            apply()
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        try {
        if (intent == null) {
            // START_STICKY restart after process death: there is no user request
            // behind this restart, so do not spin up backup work again.
            Log.w(TAG, "Restarted without intent (START_STICKY restart) — not resuming backup")
            return START_NOT_STICKY
        }
        Log.d(TAG, "Backup service started")
        Log.d(TAG, "Intent action: ${intent.action}")
        Log.d(TAG, "Backup type from intent: ${intent.getStringExtra("backup_type")}")
        Log.d(TAG, "Quick backup from intent: ${intent.getBooleanExtra("quick_backup", false)}")

        // Set backup as running in persistent storage
        setBackupRunning(true, intent.getStringExtra("backup_type") ?: "image", intent.getBooleanExtra("quick_backup", false))

        // Create notification channel for Android 8.0 and above
        createNotificationChannel()

        // Create a notification for the foreground service with HIGH priority to prevent sleep
        val contentIntent = android.app.PendingIntent.getActivity(
            this,
            0,
            android.content.Intent(this, org.openreminisce.app.MainActivity::class.java),
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
        )
        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Upload Service")
            .setContentText("Performing upload operation...")
            .setContentIntent(contentIntent)
            .setSmallIcon(R.drawable.ic_launcher_foreground) // Use app's icon instead of generic one
            .setPriority(NotificationCompat.PRIORITY_HIGH) // HIGH priority to prevent system from killing it
            .setOngoing(true) // Make it ongoing to indicate it's a foreground service
            .setCategory(NotificationCompat.CATEGORY_SERVICE) // Set proper category
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC) // Make notification visible on lock screen
            .build()
        
        try {
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
                startForeground(NOTIFICATION_ID, notification, android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
        } catch (e: Exception) {
            // Android 12+ forbids FGS starts from the background (e.g. a
            // START_STICKY restart with the app killed). The WorkManager job
            // below still runs — don't crash the service before enqueueing it.
            Log.e(TAG, "startForeground rejected (app in background?) — continuing without FGS", e)
        }
        
        } catch (t: Throwable) {
            android.util.Log.e(TAG, "onStartCommand crashed", t)
            org.openreminisce.app.util.LogCollector.e(TAG, "Service start CRASHED: ${t.javaClass.name}: ${t.message}")
            for (el in t.stackTrace.take(15)) {
                org.openreminisce.app.util.LogCollector.e(TAG, "    at ${el.className}.${el.methodName}(${el.fileName}:${el.lineNumber})")
            }
            stopSelf()
        }
        // Start the actual backup work using WorkManager
        try {
            startBackupWork(intent)
        } catch (t: Throwable) {
            // Never take the whole process down from here — log for Share Logs
            Log.e(TAG, "Failed to start backup work", t)
            org.openreminisce.app.util.LogCollector.e(TAG, "Failed to start backup work: ${t.javaClass.name}: ${t.message}")
            stopSelf()
        }
        
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

    // WorkManager delivers NULL WorkInfo once a tracked id leaves its database
    // (ExistingWorkPolicy.REPLACE, pruneWork, cancelAllWork), so the observer
    // parameter must be nullable — a non-null Kotlin parameter crashes the
    // process with an intrinsic NullPointerException on that null delivery.
    //
    // Every getWorkInfoByIdLiveData() call returns a NEW LiveData wrapper around
    // the same Room query. removeObserver() only works on the exact instance
    // that observeForever() was called on, so we store the subscribed instance;
    // otherwise observers silently leak across backup runs and receive stale
    // (null) emissions from a previous run's work id — the instant second-start
    // crash.
    private var observedWorkLiveData: androidx.lifecycle.LiveData<androidx.work.WorkInfo>? = null
    private val workStatusObserver =
        androidx.lifecycle.Observer<androidx.work.WorkInfo?> { workInfo ->
            if (workInfo == null) {
                Log.d(TAG, "Work info unavailable (work replaced/pruned) — ignoring")
                return@Observer
            }
            onWorkInfoUpdated(workInfo)
        }
    private val handler = android.os.Handler(android.os.Looper.getMainLooper())

    @Volatile
    private var completionHandled = false

    // Single background poller (replaces thread-per-poll); LiveData observer stays the primary path.
    private var pollExecutor: java.util.concurrent.ScheduledExecutorService? = null

    private fun startPolling() {
        if (pollExecutor != null) return
        pollExecutor = java.util.concurrent.Executors.newSingleThreadScheduledExecutor().also { executor ->
            executor.scheduleWithFixedDelay({
                currentWorkId?.let { workId ->
                    try {
                        val workInfo = WorkManager.getInstance(this@BackupService).getWorkInfoById(workId).get()
                        if (workInfo != null && workInfo.state.isFinished) {
                            Log.d(TAG, "Polling detected finished work: ${workInfo.state}")
                            // WorkManager LiveData (and removeObserver) are main-thread-only;
                            // hopping back here prevents IllegalStateException from the poller.
                            handler.post { handleWorkCompletion(workInfo) }
                        }
                    } catch (e: Exception) {
                        Log.e(TAG, "Error polling work status", e)
                    }
                }
            }, 10, 5, TimeUnit.SECONDS) // Start polling after 10 seconds, check every 5 seconds
        }
    }

    private fun stopPolling() {
        pollExecutor?.let {
            it.shutdownNow()
        }
        pollExecutor = null
    }

    private fun startBackupWork(intent: Intent?) {
        val backupType = intent?.getStringExtra("backup_type") ?: "image"
        val quickBackup = intent?.getBooleanExtra("quick_backup", false) ?: false

        Log.d(TAG, "Starting backup work - Type: $backupType, Quick: $quickBackup")

        // Reset so the observer + poller can't double-fire across runs
        completionHandled = false

        val constraints = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.CONNECTED)  // Backup needs the home server; without a network the run would fail instantly. The work stays enqueued until connectivity returns.
            .build()

        val inputData = Data.Builder()
            .putString("backup_type", backupType)
            .putBoolean("quick_backup", quickBackup)
            .build()

        val backupWorkRequest = OneTimeWorkRequestBuilder<BackupWorker>()
            .setConstraints(constraints)
            .setInputData(inputData)
            .addTag("backup_work")  // Add tag for tracking
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS) // Retry transient failures with backoff
            .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST) // Keep wake lock during execution
            .build()

        // Store the work ID so we can cancel it later
        currentWorkId = backupWorkRequest.id

        getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
            .edit()
            .putLong(KEY_BACKUP_RUNNING_UNTIL, System.currentTimeMillis() + BACKUP_RUNNING_WINDOW_MS)
            .apply()

        Log.d(TAG, "Enqueuing backup work request with ID: ${backupWorkRequest.id}")
        WorkManager.getInstance(this).enqueueUniqueWork(
            "reminisce_backup_work",
            androidx.work.ExistingWorkPolicy.REPLACE,
            backupWorkRequest
        )

        // Detach from any previous run before subscribing anew. This MUST target
        // the same LiveData instance that observeForever() was called on — each
        // getWorkInfoByIdLiveData() call returns a new wrapper, and removing
        // from a different wrapper is a silent no-op (the second-start crash).
        observedWorkLiveData?.let { it.removeObserver(workStatusObserver) }

        // Subscribe to this run's work status via the shared null-safe observer
        val liveData = WorkManager.getInstance(this)
            .getWorkInfoByIdLiveData(backupWorkRequest.id)
        observedWorkLiveData = liveData
        liveData.observeForever(workStatusObserver)
        Log.d(TAG, "Attached work info observer")

        // Start polling as a backup mechanism in case observeForever stops working
        startPolling()
    }

    private fun onWorkInfoUpdated(workInfo: WorkInfo) {
        Log.d(TAG, "Work info updated: ${workInfo.state}")
        Log.d(TAG, "Work ID: ${workInfo.id}")
        Log.d(TAG, "Work tags: ${workInfo.tags.joinToString(", ")}")
        Log.d(TAG, "Work run attempt count: ${workInfo.runAttemptCount}")
        Log.d(TAG, "Work output data: ${workInfo.outputData}")

        // Handle work completion
        if (workInfo.state.isFinished) {
            handleWorkCompletion(workInfo)
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

    private fun handleWorkCompletion(workInfo: WorkInfo, @Suppress("UNUSED_PARAMETER") backupType: String? = null, quickBackup: Boolean? = null) {
        if (completionHandled) return
        completionHandled = true

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

        // Remove observer when work is finished to prevent memory leak.
        // Must remove from the exact LiveData instance we subscribed to.
        observedWorkLiveData?.let {
            it.removeObserver(workStatusObserver)
            Log.d(TAG, "Removed work info observer after completion")
        }
        observedWorkLiveData = null

        // Stop polling
        stopPolling()

        Log.d(TAG, "Stopping backup service")
        // Clear the backup running state
        setBackupRunning(false)
        stopSelf()
    }

    override fun onDestroy() {
        super.onDestroy()
        // Stop the poller
        stopPolling()
        // Clean up observer when service is destroyed — again, only the stored
        // instance removal is effective.
        observedWorkLiveData?.let {
            it.removeObserver(workStatusObserver)
            Log.d(TAG, "Removed work info observer in onDestroy")
        }
        observedWorkLiveData = null
    }
}