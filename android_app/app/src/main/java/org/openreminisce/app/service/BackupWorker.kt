package org.openreminisce.app.service

import android.content.Context
import android.util.Log
import androidx.work.Worker
import androidx.work.WorkerParameters
import org.openreminisce.app.util.LogCollector
import org.openreminisce.app.util.*
import org.openreminisce.app.model.ImageInfo
import okhttp3.*
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.RequestBody.Companion.toRequestBody
import java.io.File
import androidx.core.net.toUri
import java.util.concurrent.atomic.AtomicLong
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

data class BackupStats(
    val successfullyBackedUp: Int,
    val totalProcessed: Int,
    val skippedExisting: Int,  // Files that already exist on server
    val failedCount: Int,       // Files that failed to upload
    val failedFiles: List<String> = emptyList() // Names of files that failed
)

class BackupWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    companion object {
        private const val TAG = "BackupWorker"
        private const val PROGRESS_UPDATE_THROTTLE_MS = 200L // Send progress updates frequently for responsive UI (200ms ~5 updates/sec)
        private const val NOTIFICATION_ID = 2
        private const val CHANNEL_ID = "backup_worker_channel"
        private const val WAKE_LOCK_TAG = "Reminisce::BackupWorkerWakeLock"
    }

    private var backupStats: BackupStats? = null
    @Volatile
    private var activeCall: Call? = null
    private var wakeLock: android.os.PowerManager.WakeLock? = null
    private var cpuWakeLock: android.os.PowerManager.WakeLock? = null

    // Throttling mechanism for progress updates using lock-free AtomicLong
    private val lastProgressUpdateTime = AtomicLong(0L)

    override fun onStopped() {
        super.onStopped()
        Log.d(TAG, "Worker stopped, cancelling active HTTP call")
        // Cancel any active HTTP request immediately
        activeCall?.cancel()

        // Release wake lock if held
        releaseWakeLock()
    }

    private fun acquireWakeLock() {
        try {
            val powerManager = applicationContext.getSystemService(Context.POWER_SERVICE) as android.os.PowerManager

            // NUCLEAR OPTION for Honor/Huawei devices with extremely aggressive battery management
            // Use SCREEN_BRIGHT_WAKE_LOCK to keep screen fully on during backup
            // This is the most aggressive wake lock available and most likely to work on Honor devices
            // Screen will stay on but user can still lock device normally after backup completes
            @Suppress("DEPRECATION")
            wakeLock = powerManager.newWakeLock(
                android.os.PowerManager.SCREEN_BRIGHT_WAKE_LOCK or
                android.os.PowerManager.ACQUIRE_CAUSES_WAKEUP or
                android.os.PowerManager.ON_AFTER_RELEASE,
                WAKE_LOCK_TAG
            ).apply {
                setReferenceCounted(false)
                acquire(10 * 60 * 60 * 1000L) // 10 hours max timeout
            }

            // Also acquire a separate CPU wake lock for double protection
            cpuWakeLock = powerManager.newWakeLock(
                android.os.PowerManager.PARTIAL_WAKE_LOCK,
                "${WAKE_LOCK_TAG}:CPU"
            ).apply {
                setReferenceCounted(false)
                acquire(10 * 60 * 60 * 1000L)
            }

            Log.d(TAG, "ULTRA-AGGRESSIVE wake locks acquired (SCREEN_BRIGHT + PARTIAL) - Honor/OEM extreme mode")
            Log.d(TAG, "Screen will stay ON during backup to prevent Honor battery manager from killing the process")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to acquire wake lock", e)
        }
    }

    private fun releaseWakeLock() {
        try {
            wakeLock?.let {
                if (it.isHeld) {
                    it.release()
                    Log.d(TAG, "Screen wake lock released")
                }
            }
            wakeLock = null

            cpuWakeLock?.let {
                if (it.isHeld) {
                    it.release()
                    Log.d(TAG, "CPU wake lock released")
                }
            }
            cpuWakeLock = null
        } catch (e: Exception) {
            Log.e(TAG, "Failed to release wake lock", e)
        }
    }

    override fun getForegroundInfo(): androidx.work.ForegroundInfo {
        return createForegroundInfo("Starting upload...", 0, 0)
    }

    private fun createForegroundInfo(currentFile: String, fileIndex: Int, totalFiles: Int): androidx.work.ForegroundInfo {
        // Create notification channel for Android 8.0 and above
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
            val channel = android.app.NotificationChannel(
                CHANNEL_ID,
                "Backup Worker Channel",
                android.app.NotificationManager.IMPORTANCE_HIGH // HIGH importance to prevent worker from sleeping
            ).apply {
                description = "Notifications for backup worker - keeps backup running with screen off"
                setShowBadge(true)
                lockscreenVisibility = android.app.Notification.VISIBILITY_PUBLIC
            }

            val notificationManager = applicationContext.getSystemService(android.app.NotificationManager::class.java)
            notificationManager?.createNotificationChannel(channel)
        }

        val progressText = if (totalFiles > 0) {
            "$fileIndex/$totalFiles files"
        } else {
            "Starting..."
        }

        val notification = androidx.core.app.NotificationCompat.Builder(applicationContext, CHANNEL_ID)
            .setContentTitle("Upload Active: $progressText")
            .setContentText(currentFile)
            .setSmallIcon(org.openreminisce.app.R.drawable.ic_launcher_foreground)
            .setPriority(androidx.core.app.NotificationCompat.PRIORITY_HIGH)
            .setOngoing(true)
            .setVisibility(androidx.core.app.NotificationCompat.VISIBILITY_PUBLIC)
            .setProgress(totalFiles, fileIndex, false) // Show progress bar
            .build()

        return if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
            androidx.work.ForegroundInfo(
                NOTIFICATION_ID,
                notification,
                android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
            )
        } else {
            androidx.work.ForegroundInfo(NOTIFICATION_ID, notification)
        }
    }

    // Update notification with current progress - call this frequently to prove work is ongoing
    private fun updateNotificationProgress(currentFile: String, fileIndex: Int, totalFiles: Int) {
        try {
            setForegroundAsync(createForegroundInfo(currentFile, fileIndex, totalFiles))
        } catch (e: Exception) {
            Log.e(TAG, "Failed to update notification", e)
        }
    }

    /**
     * Throttled version of setProgressAsync that only sends updates every PROGRESS_UPDATE_THROTTLE_MS.
     * Use forceUpdate = true for important progress updates that should not be throttled.
     * Uses lock-free AtomicLong for better performance under high contention.
     */
    private fun setProgressThrottled(data: androidx.work.Data, forceUpdate: Boolean = false) {
        if (forceUpdate) {
            setProgressAsync(data)
            lastProgressUpdateTime.set(System.currentTimeMillis())
            return
        }

        val currentTime = System.currentTimeMillis()
        val lastUpdate = lastProgressUpdateTime.get()

        // Only update if enough time has passed since last update
        if (currentTime - lastUpdate >= PROGRESS_UPDATE_THROTTLE_MS) {
            // Use compareAndSet to avoid race conditions
            if (lastProgressUpdateTime.compareAndSet(lastUpdate, currentTime)) {
                setProgressAsync(data)
            }
        }
    }

    override fun doWork(): Result {
        Log.d(TAG, "Backup worker started")

        // Acquire wake lock to prevent device sleep during backup
        acquireWakeLock()

        return try {
            // Promote to foreground service to prevent being killed during sleep
            try {
                setForegroundAsync(getForegroundInfo())
                Log.d(TAG, "Worker promoted to foreground service")
            } catch (e: Exception) {
                Log.e(TAG, "Failed to promote worker to foreground", e)
            }

            Log.d(TAG, "Input data processed")
            val backupType = inputData.getString("backup_type") ?: "image"
            Log.d(TAG, "Input backup type: $backupType")
            val quickBackup = inputData.getBoolean("quick_backup", false)
            Log.d(TAG, "Input quick backup: $quickBackup")

            try {
                val success = performBackup()
                Log.d(TAG, "performBackup returned: $success")
                if (success) {
                    Log.d(TAG, "Backup worker completed successfully")

                    // Add backup statistics to the result if available
                    val outputData = if (backupStats != null) {
                        val builder = androidx.work.Data.Builder()
                            .putInt("successfullyBackedUp", backupStats!!.successfullyBackedUp)
                            .putInt("totalProcessed", backupStats!!.totalProcessed)
                            .putInt("skippedExisting", backupStats!!.skippedExisting)
                            .putInt("failedCount", backupStats!!.failedCount)

                        // Add failed files as a string array (WorkManager Data supports string arrays)
                        if (backupStats!!.failedFiles.isNotEmpty()) {
                            builder.putStringArray("failedFiles", backupStats!!.failedFiles.toTypedArray())
                        }

                        builder.build()
                    } else {
                        androidx.work.Data.Builder().build()
                    }

                    Result.success(outputData)
                } else {
                    Log.w(TAG, "Backup worker completed with warnings (some operations may have failed)")
                    Result.failure()
                }
            } catch (e: Exception) {
                Log.e(TAG, "Backup worker failed with exception", e)
                Result.failure()
            }
        } finally {
            // Always release wake lock when work completes
            releaseWakeLock()
        }
    }

    private fun performBackup(): Boolean {
        Log.d(TAG, "Starting performBackup function")
        LogCollector.i(TAG, "=== Starting Backup ===")

        // Get authentication token
        val token = AuthHelper.getValidToken(applicationContext)
        val tokenStatus = if(token.isNullOrEmpty()) "NULL/EMPTY" else "AVAILABLE (${token?.length} chars)"
        Log.d(TAG, "Auth token retrieved: $tokenStatus")
        LogCollector.i(TAG, "Auth token: $tokenStatus")
        if (token.isNullOrEmpty()) {
            Log.e(TAG, "No valid token found")
            LogCollector.e(TAG, "ERROR: No valid token found - cannot upload")
            return false
        }

        // Get the base URL
        val baseUrl = PreferenceHelper.getServerUrl(applicationContext)
        Log.d(TAG, "Base URL retrieved: $baseUrl")
        LogCollector.i(TAG, "Server URL: $baseUrl")

        // Determine whether to backup images or videos
        val backupType = inputData.getString("backup_type") ?: "image"
        val quickBackup = inputData.getBoolean("quick_backup", false)
        Log.d(TAG, "Backup type: $backupType, Quick backup: $quickBackup")
        LogCollector.i(TAG, "Backup type: $backupType, Quick: $quickBackup")

        // Perform the actual backup based on the type
        val result = when (backupType) {
            "video" -> performMediaBackup(baseUrl, token, quickBackup, true)
            else -> performMediaBackup(baseUrl, token, quickBackup, false)
        }
        
        Log.d(TAG, "Backup operation completed with result: $result")
        return result
    }

    private fun performMediaBackup(baseUrl: String, token: String, quickBackup: Boolean, isVideo: Boolean): Boolean {
        Log.d(TAG, "Performing ${if(isVideo) "video" else "image"} backup - quickBackup: $quickBackup")
        
        // Get all media files of the requested type
        val allMedia = if (isVideo) {
            MediaHelper.getAllVideos(applicationContext)
        } else {
            MediaHelper.getAllImages(applicationContext)
        }
        
        Log.d(TAG, "Found ${allMedia.size} ${if(isVideo) "videos" else "images"} to process")
        LogCollector.i(TAG, "Found ${allMedia.size} ${if(isVideo) "videos" else "images"} on device")
        
        // Check if there are any media files to back up
        if (allMedia.isEmpty()) {
            Log.d(TAG, "No ${if(isVideo) "videos" else "images"} found to backup. Backup completed with no files processed.")
            // Still send a final progress update to indicate completion
            setProgressAsync(androidx.work.Data.Builder()
                .putFloat("overallProgress", 1.0f)
                .putString("currentAction", "completed_no_files")
                .putString("currentFile", "No files to backup")
                .putInt("fileIndex", 0)
                .putInt("totalFiles", 0)
                .build())
            return true
        }
        
        val databaseHelper = DatabaseHelper(applicationContext)

        // Log the device ID being used - critical for debugging skip issues
        val currentDeviceId = AuthHelper.getDeviceId(applicationContext)
        LogCollector.i(TAG, "=== DEVICE ID CHECK ===")
        LogCollector.i(TAG, "Current Device ID: $currentDeviceId")
        LogCollector.i(TAG, "This ID must match database records for files to be recognized as existing")

        // Use centralized HTTP client with long timeouts for file uploads
        val client = AuthenticatedHttpClient.getClientWithTimeouts(
            applicationContext,
            connectTimeoutSeconds = 30,
            readTimeoutSeconds = 300  // 5 minutes for large file uploads
        )
        
        // Apply quick backup filter if requested
        val mediaToBackup = if (quickBackup) {
            val lastBackupTimestamp = if (isVideo) {
                val timestamp = databaseHelper.getLastVideoBackupTimestamp()
                Log.d(TAG, "Retrieved last VIDEO backup timestamp: $timestamp")
                timestamp
            } else {
                val timestamp = databaseHelper.getLastImageBackupTimestamp()
                Log.d(TAG, "Retrieved last IMAGE backup timestamp: $timestamp")
                timestamp
            }

            if (lastBackupTimestamp != null && lastBackupTimestamp > 0) {
                val dateStr = java.text.SimpleDateFormat("yyyy-MM-dd HH:mm:ss", java.util.Locale.getDefault()).format(java.util.Date(lastBackupTimestamp * 1000))
                Log.d(TAG, "Quick ${if(isVideo) "video" else "image"} backup: Filtering files newer than timestamp $lastBackupTimestamp ($dateStr)")
                val filteredMedia = allMedia.filter { media ->
                    media.date.time / 1000 > lastBackupTimestamp // Convert to seconds
                }
                Log.d(TAG, "Quick ${if(isVideo) "video" else "image"} backup: Filtered to ${filteredMedia.size} recent files (out of ${allMedia.size} total)")
                filteredMedia
            } else {
                Log.d(TAG, "Quick ${if(isVideo) "video" else "image"} backup: No previous backup timestamp found (timestamp=$lastBackupTimestamp), backing up all ${allMedia.size} files")
                allMedia
            }
        } else {
            Log.d(TAG, "Full ${if(isVideo) "video" else "image"} backup: Backing up all ${allMedia.size} files")
            allMedia
        }
        
        // Save the backup start timestamp at the beginning of the backup (for both full and quick backups)
        // This ensures subsequent quick backups only process files newer than this backup's start time
        val startTime = System.currentTimeMillis() / 1000 // Unix timestamp in seconds
        if (isVideo) {
            Log.d(TAG, "${if(quickBackup) "Quick" else "Full"} video backup started, saving start timestamp: $startTime")
            databaseHelper.saveLastVideoBackupTimestamp(startTime)
        } else {
            Log.d(TAG, "${if(quickBackup) "Quick" else "Full"} image backup started, saving start timestamp: $startTime")
            databaseHelper.saveLastImageBackupTimestamp(startTime)
        }

        var successfullyBackedUp = 0
        var skippedExisting = 0
        var failedCount = 0
        val failedFiles = mutableListOf<String>()
        val totalToBackup = mediaToBackup.size

        // ===== PHASE 1: Quick Pre-scan with Cached Hashes =====
        Log.d(TAG, "Phase 1: Pre-scanning ${mediaToBackup.size} files with cached hashes")
        LogCollector.i(TAG, "Phase 1: Pre-scanning ${mediaToBackup.size} files")

        // Get all cached hashes from database
        val allCachedHashes = databaseHelper.getAllCachedHashes()
        Log.d(TAG, "Found ${allCachedHashes.size} cached hashes in database")

        // Map media items to their cached hashes (if valid based on modified date)
        val mediaWithValidCache = mutableMapOf<String, Pair<ImageInfo, String>>() // mediaId -> (ImageInfo, hash)
        val mediaWithoutCache = mutableListOf<ImageInfo>()

        var scanIndex = 0
        val scanTotal = mediaToBackup.size
        for (mediaInfo in mediaToBackup) {
            scanIndex++

            // Update progress every 100 files during scanning
            if (scanIndex % 100 == 0 || scanIndex == scanTotal) {
                setProgressThrottled(androidx.work.Data.Builder()
                    .putFloat("overallProgress", scanIndex.toFloat() / scanTotal.toFloat() * 0.1f) // Scanning is 10% of total
                    .putString("currentAction", "scanning")
                    .putString("currentFile", "Scanning files: $scanIndex / $scanTotal")
                    .putInt("fileIndex", scanIndex)
                    .putInt("totalFiles", scanTotal)
                    .putInt("backedUpCount", 0)
                    .putInt("skippedCount", 0)
                    .putInt("failedCount", 0)
                    .build())
            }

            val fileUri = mediaInfo.id.toUri()
            val fileModifiedDate = MediaHelper.getLastModifiedFromUri(applicationContext, fileUri)

            if (fileModifiedDate != null) {
                val cachedInfo = allCachedHashes[mediaInfo.id]
                if (cachedInfo != null && cachedInfo.modifiedDate == fileModifiedDate) {
                    // Cache hit with valid modified date
                    mediaWithValidCache[mediaInfo.id] = Pair(mediaInfo, cachedInfo.hash)
                } else {
                    // Cache miss or modified file
                    mediaWithoutCache.add(mediaInfo)
                }
            } else {
                // Can't get modified date, add to uncached
                mediaWithoutCache.add(mediaInfo)
            }
        }

        Log.d(TAG, "Phase 1: ${mediaWithValidCache.size} files with valid cache, ${mediaWithoutCache.size} files need hash calculation")

        // Build file metadata for cached files (hash + full path)
        val cachedFilesMetadata = mutableListOf<FileMetadata>()
        val hashToMediaInfo = mutableMapOf<String, ImageInfo>()

        var preparedCount = 0
        val totalCached = mediaWithValidCache.size
        
        Log.d(TAG, "Phase 1: Preparing metadata for $totalCached cached files")
        
        for ((mediaId, pair) in mediaWithValidCache) {
            preparedCount++
            
            // Update progress every 200 files
            if (preparedCount % 200 == 0 || preparedCount == totalCached) {
                setProgressThrottled(androidx.work.Data.Builder()
                    .putFloat("overallProgress", 0.1f) // Keep at 10% while preparing
                    .putString("currentAction", "analyzing")
                    .putString("currentFile", "Preparing metadata: $preparedCount/$totalCached")
                    .putInt("fileIndex", preparedCount)
                    .putInt("totalFiles", totalToBackup)
                    .putInt("backedUpCount", 0)
                    .putInt("skippedCount", 0)
                    .putInt("failedCount", 0)
                    .build())
            }
            
            val (mediaInfo, hash) = pair
            
            // OPTIMIZATION: Use cached path info if available to avoid expensive DB queries
            val fullPath = if (mediaInfo.displayName != null) {
                // Use cached info
                val relative = mediaInfo.relativePath ?: ""
                relative + mediaInfo.displayName
            } else {
                // Fallback to slow DB query
                val fileUri = mediaInfo.id.toUri()
                MediaHelper.getFullPathFromUri(applicationContext, fileUri)
            }
            
            if (fullPath != null) {
                cachedFilesMetadata.add(FileMetadata(hash, fullPath))
                hashToMediaInfo[hash] = mediaInfo
            }
        }

        LogCollector.i(TAG, "Batch checking ${cachedFilesMetadata.size} cached files with server...")

        // Update progress to show we're now checking with server
        setProgressAsync(androidx.work.Data.Builder()
            .putFloat("overallProgress", 0.1f) // Scanning done (10%), now checking (10-20%)
            .putString("currentAction", "checking_server")
            .putString("currentFile", "${cachedFilesMetadata.size} files")
            .putInt("fileIndex", 0)
            .putInt("totalFiles", totalToBackup)
            .putInt("backedUpCount", 0)
            .putInt("skippedCount", 0)
            .putInt("failedCount", 0)
            .build())

        // Batch check with metadata - server handles deduplication
        // We handle chunking explicitly here to provide progress updates
        var phase1Result = BatchCheckResult.EMPTY
        val phase1Chunks = cachedFilesMetadata.chunked(100)
        var phase1CheckedCount = 0
        
        Log.d(TAG, "Phase 1: checking ${cachedFilesMetadata.size} files in ${phase1Chunks.size} chunks")

        for ((chunkIndex, chunk) in phase1Chunks.withIndex()) {
            // Check for cancellation
            if (isStopped) break
            
            // Update progress before check
            setProgressThrottled(androidx.work.Data.Builder()
                .putFloat("overallProgress", 0.1f + (phase1CheckedCount.toFloat() / cachedFilesMetadata.size.toFloat() * 0.1f)) // 10-20% range
                .putString("currentAction", "checking_server")
                .putString("currentFile", "${phase1CheckedCount}/${cachedFilesMetadata.size} files")
                .putInt("fileIndex", phase1CheckedCount) // Show how many checked so far
                .putInt("totalFiles", totalToBackup)
                .putInt("backedUpCount", 0)
                .putInt("skippedCount", 0)
                .putInt("failedCount", 0)
                .build(), forceUpdate = true)
                
            val chunkResult = batchCheckFiles(chunk, baseUrl, token, if(isVideo) "video" else "image", client)
            phase1Result += chunkResult
            phase1CheckedCount += chunk.size
            
            Log.d(TAG, "Phase 1 chunk ${chunkIndex+1}/${phase1Chunks.size}: ${chunkResult.existsForDevice.size} exist, ${chunkResult.deduplicated.size} dedup, ${chunkResult.needsUpload.size} need upload")
        }

        Log.d(TAG, "Phase 1: ${phase1Result.existsForDevice.size} exist for device, ${phase1Result.deduplicated.size} deduplicated, ${phase1Result.needsUpload.size} need upload")
        LogCollector.i(TAG, "Batch check: ${phase1Result.existsForDevice.size} exist, ${phase1Result.deduplicated.size} dedup, ${phase1Result.needsUpload.size} upload")

        // Detailed summary for debugging
        val totalChecked = phase1Result.existsForDevice.size + phase1Result.deduplicated.size + phase1Result.needsUpload.size
        val willSkip = phase1Result.existsForDevice.size + phase1Result.deduplicated.size
        val willUpload = phase1Result.needsUpload.size
        LogCollector.i(TAG, "=== PHASE 1 SUMMARY ===")
        LogCollector.i(TAG, "Total files checked: $totalChecked")
        LogCollector.i(TAG, "Will SKIP (already on server): $willSkip")
        LogCollector.i(TAG, "Will UPLOAD (new files): $willUpload")
        LogCollector.i(TAG, "========================")

        // Count skipped and deduplicated files
        // We treat deduplicated files as "skipped" for the UI progress because they don't require data transfer
        skippedExisting = phase1Result.existsForDevice.size + phase1Result.deduplicated.size
        successfullyBackedUp = 0 // Will increment only for actual file uploads

        // Show individual "skipped" progress for Phase 1 cached files
        var phase1SkippedIndex = 0
        val allPhase1SkippedHashes = phase1Result.existsForDevice + phase1Result.deduplicated
        for (skippedHash in allPhase1SkippedHashes) {
            phase1SkippedIndex++

            // Check for cancellation
            if (isStopped) break

            val mediaInfo = hashToMediaInfo[skippedHash]
            if (mediaInfo != null) {
                val fileName = mediaInfo.displayName
                    ?: MediaHelper.getDisplayNameFromUri(applicationContext, mediaInfo.id.toUri())
                    ?: "Unknown"

                setProgressThrottled(androidx.work.Data.Builder()
                    .putFloat("overallProgress", phase1SkippedIndex.toFloat() / totalToBackup.toFloat())
                    .putString("currentAction", "skipped")
                    .putString("currentFile", fileName)
                    .putInt("fileIndex", phase1SkippedIndex)
                    .putInt("totalFiles", totalToBackup)
                    .putInt("backedUpCount", successfullyBackedUp)
                    .putInt("skippedCount", phase1SkippedIndex)
                    .putInt("failedCount", failedCount)
                    .build())
            }
        }

        // Force a final progress update after Phase 1 skipped files
        if (skippedExisting > 0) {
            setProgressAsync(androidx.work.Data.Builder()
                .putFloat("overallProgress", skippedExisting.toFloat() / totalToBackup.toFloat())
                .putString("currentAction", "pre_scan_complete")
                .putString("currentFile", "Pre-scan complete: $skippedExisting already on server")
                .putInt("fileIndex", skippedExisting)
                .putInt("totalFiles", totalToBackup)
                .putInt("backedUpCount", successfullyBackedUp)
                .putInt("skippedCount", skippedExisting)
                .putInt("failedCount", failedCount)
                .build())
        }

        // ===== PHASE 2: Process files needing upload =====
        // Two types of files:
        // 1. Files from Phase 1 - anything NOT confirmed to exist on server
        // 2. Uncached files - need hash calculation, batch check, then upload

        // Track files ready for immediate upload (from Phase 1)
        data class FileReadyForUpload(
            val mediaInfo: ImageInfo,
            val hash: String,
            val fullPath: String
        )
        val filesReadyForUpload = mutableListOf<FileReadyForUpload>()

        // Add all cached files that are NOT explicitly skipped (already on server)
        var prepareUploadCount = 0
        val confirmedSkippedHashes = phase1Result.existsForDevice + phase1Result.deduplicated
        
        Log.d(TAG, "Phase 2: Preparing upload list. confirmedSkipped=${confirmedSkippedHashes.size}, totalCached=${mediaWithValidCache.size}")
        
        for ((mediaId, pair) in mediaWithValidCache) {
            val (mediaInfo, hash) = pair
            
            // If the server didn't confirm this file exists, we must upload it
            if (!confirmedSkippedHashes.contains(hash)) {
                prepareUploadCount++
                
                // Update progress every 100 files
                if (prepareUploadCount % 100 == 0) {
                    setProgressThrottled(androidx.work.Data.Builder()
                        .putFloat("overallProgress", 0.2f) // Scanning/checking done
                        .putString("currentAction", "analyzing")
                        .putString("currentFile", "Preparing upload: $prepareUploadCount")
                        .putInt("fileIndex", skippedExisting)
                        .putInt("totalFiles", totalToBackup)
                        .putInt("backedUpCount", 0)
                        .putInt("skippedCount", skippedExisting)
                        .putInt("failedCount", 0)
                        .build())
                }

                // OPTIMIZATION: Use cached path info if available
                val fullPath = if (mediaInfo.displayName != null) {
                    (mediaInfo.relativePath ?: "") + mediaInfo.displayName
                } else {
                    MediaHelper.getFullPathFromUri(applicationContext, mediaInfo.id.toUri())
                }
                
                if (fullPath != null) {
                    filesReadyForUpload.add(FileReadyForUpload(mediaInfo, hash, fullPath))
                }
            }
        }

        Log.d(TAG, "Phase 2: ${filesReadyForUpload.size} files ready for upload (from cached), ${mediaWithoutCache.size} files need hash calculation")

        var processedCount = skippedExisting + successfullyBackedUp // Start from already processed count

        // Part A: Upload files that are ready (from Phase 1 needsUpload)
        LogCollector.i(TAG, "=== STARTING UPLOADS ===")
        LogCollector.i(TAG, "Files to upload from Phase 1: ${filesReadyForUpload.size}")
        LogCollector.i(TAG, "Files already skipped: $skippedExisting")
        LogCollector.i(TAG, "Uncached files to process: ${mediaWithoutCache.size}")

        for ((index, file) in filesReadyForUpload.withIndex()) {
            // Check if cancelled
            val prefs = applicationContext.getSharedPreferences("BackupState", android.content.Context.MODE_PRIVATE)
            val cancelRequested = prefs.getBoolean("cancel_backup", false)
            if (isStopped || cancelRequested) {
                Log.d(TAG, "Backup cancelled during upload")
                prefs.edit().putBoolean("cancel_backup", false).apply()
                return false
            }

            processedCount++
            val fileUri = file.mediaInfo.id.toUri()
            val fileName = MediaHelper.getDisplayNameFromUri(applicationContext, fileUri)
            val fileSize = MediaHelper.getFileSizeFromUri(applicationContext, fileUri)

            if (fileName == null || fileSize == null) {
                Log.e(TAG, "Could not get file metadata for: ${file.mediaInfo.id}")
                failedCount++
                failedFiles.add(file.mediaInfo.id)
                continue
            }

            Log.d(TAG, "Uploading file ${index + 1}/${filesReadyForUpload.size}: $fileName")

            val dateTakenMs = MediaHelper.getDateTakenFromUri(applicationContext, fileUri)
                ?: MediaHelper.getLastModifiedFromUri(applicationContext, fileUri)?.let { it * 1000L }
            val uploadSuccess = uploadFileWithProgress(
                uri = fileUri,
                fileName = fileName,
                fullPath = file.fullPath,
                type = if (isVideo) "video" else "image",
                fileHash = file.hash,
                baseUrl = baseUrl,
                token = token,
                client = client,
                overallProgress = processedCount.toFloat() / totalToBackup.toFloat(),
                totalFiles = totalToBackup,
                fileIndex = processedCount,
                backedUpCount = successfullyBackedUp,
                skippedCount = skippedExisting,
                failedCount = failedCount,
                fileSize = fileSize,
                dateTakenMs = dateTakenMs,
            )

            if (uploadSuccess) {
                successfullyBackedUp++
                // Hash is already cached from Phase 1
            } else {
                failedCount++
                failedFiles.add(fileName)
                Log.e(TAG, "Failed to upload $fileName")
            }

            // Update progress
            if (!isStopped) {
                setProgressThrottled(androidx.work.Data.Builder()
                    .putFloat("overallProgress", processedCount.toFloat() / totalToBackup.toFloat())
                    .putString("currentAction", if (uploadSuccess) "uploaded" else "upload_failed")
                    .putString("currentFile", fileName)
                    .putInt("fileIndex", processedCount)
                    .putInt("totalFiles", totalToBackup)
                    .putInt("backedUpCount", successfullyBackedUp)
                    .putInt("skippedCount", skippedExisting)
                    .putInt("failedCount", failedCount)
                    .build())

                updateNotificationProgress(fileName, processedCount, totalToBackup)
            }
        }

        // Part B: Process uncached files in chunks (need hash calculation and batch check)
        val chunkSize = 50 // Process 50 files at a time
        val uncachedChunks = mediaWithoutCache.chunked(chunkSize)

        for ((chunkIndex, chunk) in uncachedChunks.withIndex()) {
            Log.d(TAG, "Processing uncached chunk ${chunkIndex + 1}/${uncachedChunks.size} (${chunk.size} files)")

            // Check if cancelled
            val prefs = applicationContext.getSharedPreferences("BackupState", android.content.Context.MODE_PRIVATE)
            val cancelRequested = prefs.getBoolean("cancel_backup", false)
            if (isStopped || cancelRequested) {
                Log.d(TAG, "Backup cancelled during chunk processing")
                prefs.edit().putBoolean("cancel_backup", false).apply()
                return false
            }

            // Step 1: Calculate hashes for files in this chunk
            data class ChunkFile(
                val mediaInfo: ImageInfo,
                val hash: String,
                val fullPath: String,
                val modifiedDate: Long,
                val fileSize: Long,
                val dateTakenMs: Long?,  // DATE_TAKEN from MediaStore (ms), null if unavailable
            )
            val chunkWithHashes = mutableListOf<ChunkFile>()

            for (mediaInfo in chunk) {
                val fileUri = mediaInfo.id.toUri()
                
                // OPTIMIZATION: Use cached metadata if available
                val fileName = mediaInfo.displayName ?: MediaHelper.getDisplayNameFromUri(applicationContext, fileUri)
                val fullPath = if (mediaInfo.displayName != null) {
                    (mediaInfo.relativePath ?: "") + mediaInfo.displayName
                } else {
                    MediaHelper.getFullPathFromUri(applicationContext, fileUri)
                }
                
                val fileSize = MediaHelper.getFileSizeFromUri(applicationContext, fileUri)
                val fileModifiedDate = MediaHelper.getLastModifiedFromUri(applicationContext, fileUri)

                if (fileName == null || fileSize == null || fileModifiedDate == null || fullPath == null) {
                    Log.e(TAG, "Could not get file metadata for: ${mediaInfo.id}")
                    failedCount++
                    failedFiles.add(mediaInfo.id)
                    continue
                }

                val hash = try {
                    // Report progress: calculating hash
                    setProgressAsync(androidx.work.Data.Builder()
                        .putFloat("overallProgress", processedCount.toFloat() / totalToBackup.toFloat())
                        .putString("currentAction", "calculating_hash")
                        .putString("currentFile", fileName)
                        .putInt("fileIndex", processedCount + 1)
                        .putInt("totalFiles", totalToBackup)
                        .putInt("backedUpCount", successfullyBackedUp)
                        .putInt("skippedCount", skippedExisting)
                        .putInt("failedCount", failedCount)
                        .build())

                    HashCalculator.calculateHashFromUri(
                        applicationContext,
                        fileUri,
                        fileSize,
                        onProgress = { progress ->
                            if (!isStopped) {
                                setProgressThrottled(androidx.work.Data.Builder()
                                    .putFloat("overallProgress", processedCount.toFloat() / totalToBackup.toFloat())
                                    .putFloat("fileProgress", progress)
                                    .putString("currentAction", "calculating_hash")
                                    .putString("currentFile", fileName)
                                    .putInt("fileIndex", processedCount + 1)
                                    .putInt("totalFiles", totalToBackup)
                                    .putInt("backedUpCount", successfullyBackedUp)
                                    .putInt("skippedCount", skippedExisting)
                                    .putInt("failedCount", failedCount)
                                    .build())
                            }
                        },
                        shouldCancel = {
                            val cancelPrefs = applicationContext.getSharedPreferences("BackupState", android.content.Context.MODE_PRIVATE)
                            val isCancelRequested = cancelPrefs.getBoolean("cancel_backup", false)
                            isStopped || isCancelRequested
                        }
                    ).also { calculatedHash ->
                        // Cache the newly calculated hash
                        databaseHelper.insertHash(mediaInfo.id, calculatedHash, fileModifiedDate)
                        Log.d(TAG, "Cached hash for $fileName")
                    }
                } catch (e: InterruptedException) {
                    Log.d(TAG, "Hash calculation cancelled for $fileName")
                    val cancelPrefs = applicationContext.getSharedPreferences("BackupState", android.content.Context.MODE_PRIVATE)
                    cancelPrefs.edit().putBoolean("cancel_backup", false).apply()
                    return false
                } catch (e: Exception) {
                    Log.e(TAG, "Error calculating hash for $fileName", e)
                    failedCount++
                    failedFiles.add(fileName)
                    continue
                }

                val dateTakenMs = MediaHelper.getDateTakenFromUri(applicationContext, fileUri)
                    ?: (fileModifiedDate * 1000L)  // DATE_MODIFIED in seconds → ms fallback
                chunkWithHashes.add(ChunkFile(mediaInfo, hash, fullPath, fileModifiedDate, fileSize, dateTakenMs))
            }

            // Step 2: Batch check with metadata - server handles deduplication
            val filesToCheck = chunkWithHashes.map { FileMetadata(it.hash, it.fullPath) }
            val chunkBatchResult = if (filesToCheck.isNotEmpty()) {
                if (!isStopped) {
                    setProgressThrottled(androidx.work.Data.Builder()
                        .putFloat("overallProgress", processedCount.toFloat() / totalToBackup.toFloat())
                        .putString("currentAction", "checking_server")
                        .putString("currentFile", "${filesToCheck.size} files")
                        .putInt("fileIndex", processedCount + 1)
                        .putInt("totalFiles", totalToBackup)
                        .putInt("backedUpCount", successfullyBackedUp)
                        .putInt("skippedCount", skippedExisting)
                        .putInt("failedCount", failedCount)
                        .build(), forceUpdate = true)
                }
                
                Log.d(TAG, "Batch checking ${filesToCheck.size} hashes with metadata")
                batchCheckFiles(filesToCheck, baseUrl, token, if(isVideo) "video" else "image", client)
            } else {
                BatchCheckResult.EMPTY
            }

            Log.d(TAG, "Chunk ${chunkIndex + 1}: ${chunkBatchResult.existsForDevice.size} exist, ${chunkBatchResult.deduplicated.size} dedup, ${chunkBatchResult.needsUpload.size} need upload")

            // Update counters from batch check results
            skippedExisting += chunkBatchResult.existsForDevice.size + chunkBatchResult.deduplicated.size
            // successfullyBackedUp only increments for actual uploads, not deduplication

            // Step 3: Upload files (only skip if explicitly marked as existing/deduplicated)
            for (chunkFile in chunkWithHashes) {
                processedCount++

                // Skip if already exists for device or was deduplicated
                if (chunkBatchResult.existsForDevice.contains(chunkFile.hash) || chunkBatchResult.deduplicated.contains(chunkFile.hash)) {
                    val isDedup = chunkBatchResult.deduplicated.contains(chunkFile.hash)
                    Log.d(TAG, "${chunkFile.fullPath} ${if(isDedup) "deduplicated by server" else "already exists for device"}, skipping")
                    
                    if (!isStopped) {
                        setProgressThrottled(androidx.work.Data.Builder()
                            .putFloat("overallProgress", processedCount.toFloat() / totalToBackup.toFloat())
                            .putString("currentAction", "skipped")
                            .putString("currentFile", MediaHelper.getDisplayNameFromUri(applicationContext, chunkFile.mediaInfo.id.toUri()) ?: "Unknown")
                            .putInt("fileIndex", processedCount)
                            .putInt("totalFiles", totalToBackup)
                            .putInt("backedUpCount", successfullyBackedUp)
                            .putInt("skippedCount", skippedExisting)
                            .putInt("failedCount", failedCount)
                            .build())
                    }
                    continue
                }

                // If NOT in any of the skip sets, we UPLOAD it (treat uncertainty as "needs upload")
                val fileUri = chunkFile.mediaInfo.id.toUri()
                val fileName = MediaHelper.getDisplayNameFromUri(applicationContext, fileUri)

                if (fileName == null) {
                    Log.e(TAG, "Could not get filename for: ${chunkFile.mediaInfo.id}")
                    failedCount++
                    failedFiles.add(chunkFile.mediaInfo.id)
                    continue
                }

                // File is new or server didn't explicitly say it exists, upload it
                val uploadSuccess = uploadFileWithProgress(
                    uri = fileUri,
                    fileName = fileName,
                    fullPath = chunkFile.fullPath,
                    type = if (isVideo) "video" else "image",
                    fileHash = chunkFile.hash,
                    baseUrl = baseUrl,
                    token = token,
                    client = client,
                    overallProgress = processedCount.toFloat() / totalToBackup.toFloat(),
                    totalFiles = totalToBackup,
                    fileIndex = processedCount,
                    backedUpCount = successfullyBackedUp,
                    skippedCount = skippedExisting,
                    failedCount = failedCount,
                    fileSize = chunkFile.fileSize,
                    dateTakenMs = chunkFile.dateTakenMs,
                )

                if (uploadSuccess) {
                    successfullyBackedUp++
                    // Hash already cached during calculation
                } else {
                    failedCount++
                    failedFiles.add(fileName)
                    Log.e(TAG, "Failed to upload $fileName")
                }

                // Update progress after processing the file
                if (!isStopped) {
                    setProgressThrottled(androidx.work.Data.Builder()
                        .putFloat("overallProgress", processedCount.toFloat() / totalToBackup.toFloat())
                        .putString("currentAction", if (uploadSuccess) "uploaded" else "upload_failed")
                        .putString("currentFile", fileName)
                        .putInt("fileIndex", processedCount)
                        .putInt("totalFiles", totalToBackup)
                        .putInt("backedUpCount", successfullyBackedUp)
                        .putInt("skippedCount", skippedExisting)
                        .putInt("failedCount", failedCount)
                        .build())

                    updateNotificationProgress(fileName, processedCount, totalToBackup)
                    Log.d(TAG, "HEARTBEAT: Processed file $processedCount/$totalToBackup at ${System.currentTimeMillis()}")
                }
            }
        }


        // Log accurate final statistics
        Log.d(TAG, "Backup completed - Total: ${mediaToBackup.size}, Uploaded: $successfullyBackedUp, Skipped (existing): $skippedExisting, Failed: $failedCount")
        if (failedFiles.isNotEmpty()) {
            Log.e(TAG, "Failed files: ${failedFiles.joinToString(", ")}")
        }

        // Store accurate backup statistics
        backupStats = BackupStats(
            successfullyBackedUp = successfullyBackedUp,
            totalProcessed = mediaToBackup.size,
            skippedExisting = skippedExisting,
            failedCount = failedCount,
            failedFiles = failedFiles
        )

        // Always return true if we completed the loop (stats will show success/failure breakdown)
        // Only return false if we couldn't even start or had a fatal error
        return true
    }

    /**
     * File metadata for batch check request
     */
    private data class FileMetadata(
        val hash: String,
        val name: String  // Full path like "DCIM/Camera/IMG_001.jpg"
    )

    /**
     * Data class to hold batch check results
     * @param existsForDevice Hashes that exist for current device (skip upload)
     * @param deduplicated Hashes that existed for other device - server created metadata (skip upload)
     * @param needsUpload Hashes that need full upload
     */
    private data class BatchCheckResult(
        val existsForDevice: Set<String>,
        val deduplicated: Set<String>,
        val needsUpload: Set<String>
    ) {
        companion object {
            val EMPTY = BatchCheckResult(emptySet(), emptySet(), emptySet())
        }

        operator fun plus(other: BatchCheckResult): BatchCheckResult {
            return BatchCheckResult(
                existsForDevice + other.existsForDevice,
                deduplicated + other.deduplicated,
                needsUpload + other.needsUpload
            )
        }
    }

    /**
     * Batch check files with metadata - server handles deduplication automatically
     * @param files List of FileMetadata (hash + name) to check (max 100)
     * @param baseUrl Server base URL
     * @param token Authentication token
     * @param type "image" or "video"
     * @param client OkHttpClient to use
     * @return BatchCheckResult with exists_for_device, deduplicated, and needs_upload sets
     */
    private fun batchCheckFiles(
        files: List<FileMetadata>,
        baseUrl: String,
        token: String,
        type: String,
        client: OkHttpClient
    ): BatchCheckResult {
        if (files.isEmpty()) {
            return BatchCheckResult.EMPTY
        }

        if (files.size > 100) {
            Log.w(TAG, "Batch check limited to 100 files, got ${files.size}. Splitting...")
            return files.chunked(100).fold(BatchCheckResult.EMPTY) { acc, chunk ->
                acc + batchCheckFiles(chunk, baseUrl, token, type, client)
            }
        }

        val checkUrl = "${baseUrl.trimEnd('/')}/api/upload/batch-check-${type}s"
        val deviceId = AuthHelper.getDeviceId(applicationContext)

        LogCollector.i(TAG, "Batch check: ${files.size} ${type}s, DeviceID: $deviceId")
        LogCollector.d(TAG, "URL: $checkUrl")

        // Log first few hashes for debugging - FULL hashes for database comparison
        if (files.isNotEmpty()) {
            LogCollector.i(TAG, "=== SAMPLE HASHES FOR DB CHECK ===")
            files.take(3).forEach { file ->
                LogCollector.i(TAG, "Hash: ${file.hash}")
                LogCollector.i(TAG, "Name: ${file.name}")
            }
            LogCollector.i(TAG, "Check in DB: SELECT * FROM images WHERE hash = '<hash>'")
            LogCollector.i(TAG, "==================================")
        }

        // Create JSON request body with device_id and hashes array
        // Backend expects: { "device_id": "...", "hashes": ["hash1", "hash2"] }
        val hashesArray = org.json.JSONArray()
        for (file in files) {
            hashesArray.put(file.hash)
        }
        val json = org.json.JSONObject()
        json.put("device_id", deviceId)
        json.put("hashes", hashesArray)

        val requestBody = json.toString().toRequestBody(
            "application/json; charset=utf-8".toMediaTypeOrNull()
        )

        val request = Request.Builder()
            .url(checkUrl)
            .post(requestBody)
            .addHeader("Authorization", "Bearer $token")
            .addHeader("Content-Type", "application/json")
            .build()

        try {
            val response = AuthHelper.executeWithTokenRefresh(applicationContext, client, request)

            if (response.isSuccessful) {
                val responseBody = response.body?.string()
                val jsonResponse = org.json.JSONObject(responseBody ?: "{}")

                // Backend returns: { "existing_hashes": ["hash1", "hash2"] }
                val existingHashesArray = jsonResponse.optJSONArray("existing_hashes")
                val existingHashes = mutableSetOf<String>()
                
                if (existingHashesArray != null) {
                    for (i in 0 until existingHashesArray.length()) {
                        existingHashes.add(existingHashesArray.getString(i))
                    }
                }

                // Determine which files need upload
                val needsUpload = mutableSetOf<String>()
                val deduplicated = mutableSetOf<String>() // Not used by current backend, but kept for compatibility
                
                // Check each requested file against existing hashes
                for (file in files) {
                    if (!existingHashes.contains(file.hash)) {
                        needsUpload.add(file.hash)
                    }
                }

                LogCollector.i(TAG, "Batch check result: exist=${existingHashes.size}, upload=${needsUpload.size}")

                return BatchCheckResult(existingHashes, deduplicated, needsUpload)
            } else if (response.code == 401) {
                Log.e(TAG, "Authentication failed (401) during batch check")
                LogCollector.e(TAG, "Batch check FAILED (401 auth error)")
            } else if (response.code == 404) {
                Log.d(TAG, "Batch endpoint not found (404)")
                LogCollector.w(TAG, "Batch check endpoint not found (404)")
            } else {
                Log.e(TAG, "Batch check failed with code: ${response.code}")
                LogCollector.e(TAG, "Batch check FAILED (${response.code})")
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error during batch check for ${files.size} files", e)
            LogCollector.e(TAG, "Batch check EXCEPTION: ${e.javaClass.simpleName}: ${e.message}")
        }

        return BatchCheckResult.EMPTY
    }

    private fun uploadFileWithProgress(
        uri: android.net.Uri,
        fileName: String,
        fullPath: String,
        type: String,
        fileHash: String,
        baseUrl: String,
        token: String,
        client: OkHttpClient,
        overallProgress: Float,
        totalFiles: Int,
        fileIndex: Int,
        backedUpCount: Int,
        skippedCount: Int,
        failedCount: Int,
        fileSize: Long? = null,
        isRetry: Boolean = false,
        dateTakenMs: Long? = null,
    ): Boolean {
        // Check if work was cancelled before starting upload
        if (isStopped) {
            Log.d(TAG, "Upload cancelled before starting: $fileName")
            return false
        }

        Log.d(TAG, "Uploading $type: $fullPath (URI: $uri)${if (isRetry) " [RETRY with recalculated hash]" else ""}")
        val uploadUrl = "${baseUrl.trimEnd('/')}/api/upload/$type"

        // Build multipart request (thumbnail will be generated server-side)
        val deviceId = AuthHelper.getDeviceId(applicationContext)
        val requestBodyBuilder = MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart("hash", fileHash)
            .addFormDataPart("name", fullPath)
            .addFormDataPart("device_id", deviceId)

        // Send capture date (DATE_TAKEN from MediaStore) so server can use it when EXIF is absent
        if (dateTakenMs != null && dateTakenMs > 0) {
            val sdf = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US)
            sdf.timeZone = TimeZone.getTimeZone("UTC")
            requestBodyBuilder.addFormDataPart("created_at", sdf.format(Date(dateTakenMs)))
        }

        // Add the main file using URI
        requestBodyBuilder.addFormDataPart(
            type,
            fileName,
            ProgressRequestBody.fromUri(applicationContext, uri, fileName, object : ProgressRequestBody.UploadCallbacks {
                override fun onProgressUpdate(percentage: Int) {
                    if (!isStopped) {
                        // Use throttled updates for upload progress (called frequently)
                        setProgressThrottled(androidx.work.Data.Builder()
                            .putFloat("fileUploadProgress", percentage.toFloat() / 100f)
                            .putFloat("overallProgress", overallProgress)
                            .putString("currentAction", "uploading")
                            .putString("currentFile", fileName)
                            .putInt("totalFiles", totalFiles)
                            .putInt("fileIndex", fileIndex)
                            .putInt("backedUpCount", backedUpCount)
                            .putInt("skippedCount", skippedCount)
                            .putInt("failedCount", failedCount)
                            .build())
                    }
                }
            })
        )

        val requestBody = requestBodyBuilder.build()

        Log.d("BackupWorker", "Uploading $type to server with Device ID: $deviceId, file: $fileName")
        val request = Request.Builder()
            .url(uploadUrl)
            .post(requestBody)
            .addHeader("Authorization", "Bearer $token")
            .build()

        try {
            val call = client.newCall(request)
            // Store the active call so it can be cancelled if needed
            activeCall = call

            val response = call.execute()

            // Clear the active call reference
            activeCall = null

            // Check if cancelled after upload completed but before processing response
            if (isStopped) {
                Log.d(TAG, "Upload cancelled after completion: $fileName")
                return false
            }

            if (response.isSuccessful) {
                Log.d(TAG, "File uploaded successfully: $fileName${if (isRetry) " (after retry)" else ""}")
                LogCollector.i(TAG, "Uploaded: $fileName")
                return true
            } else if (response.code == 401) {
                Log.e(TAG, "Authentication failed (401) during file upload")
                LogCollector.e(TAG, "Upload FAILED (401 auth error): $fileName")
                return false
            } else if (response.code == 400) {
                val responseBody = response.body?.string()
                Log.e(TAG, "Upload failed with 400 Bad Request: $responseBody for $fileName")

                // Try to parse the error response to check if it's a hash verification failure
                try {
                    val json = org.json.JSONObject(responseBody ?: "{}")
                    val status = json.optString("status", "")
                    val message = json.optString("message", "")

                    if (status == "error" && message.contains("Hash verification failed", ignoreCase = true)) {
                        val expectedHash = json.optString("expected_hash", "unknown")
                        val calculatedHash = json.optString("calculated_hash", "unknown")

                        Log.e(TAG, "HASH VERIFICATION FAILED for $fileName:")
                        Log.e(TAG, "  - Client calculated hash: $fileHash")
                        Log.e(TAG, "  - Server expected hash: $expectedHash")
                        Log.e(TAG, "  - Server calculated hash: $calculatedHash")

                        // Invalidate the cached hash for this file since it appears to be incorrect
                        val mediaId = uri.lastPathSegment ?: uri.toString()
                        val databaseHelper = DatabaseHelper(applicationContext)
                        databaseHelper.deleteHash(mediaId)
                        Log.w(TAG, "Invalidated cached hash for $fileName")

                        // Only retry once to avoid infinite loops
                        if (!isRetry && fileSize != null) {
                            Log.w(TAG, "Recalculating hash and retrying upload for $fileName")

                            // Recalculate the hash
                            val recalculatedHash = try {
                                HashCalculator.calculateHashFromUri(
                                    applicationContext,
                                    uri,
                                    fileSize,
                                    onProgress = { progress ->
                                        if (!isStopped) {
                                            setProgressThrottled(androidx.work.Data.Builder()
                                                .putFloat("overallProgress", overallProgress)
                                                .putFloat("fileProgress", progress)
                                                .putString("currentAction", "recalculating_hash")
                                                .putString("currentFile", fileName)
                                                .putInt("fileIndex", fileIndex)
                                                .putInt("totalFiles", totalFiles)
                                                .putInt("backedUpCount", backedUpCount)
                                                .putInt("skippedCount", skippedCount)
                                                .putInt("failedCount", failedCount)
                                                .build())
                                        }
                                    },
                                    shouldCancel = {
                                        val cancelPrefs = applicationContext.getSharedPreferences("BackupState", android.content.Context.MODE_PRIVATE)
                                        val isCancelRequested = cancelPrefs.getBoolean("cancel_backup", false)
                                        isStopped || isCancelRequested
                                    }
                                )
                            } catch (e: Exception) {
                                Log.e(TAG, "Failed to recalculate hash for $fileName", e)
                                return false
                            }

                            Log.d(TAG, "Recalculated hash for $fileName: $recalculatedHash")

                            // Retry the upload with the recalculated hash
                            return uploadFileWithProgress(
                                uri = uri,
                                fileName = fileName,
                                fullPath = fullPath,
                                type = type,
                                fileHash = recalculatedHash,
                                baseUrl = baseUrl,
                                token = token,
                                client = client,
                                overallProgress = overallProgress,
                                totalFiles = totalFiles,
                                fileIndex = fileIndex,
                                backedUpCount = backedUpCount,
                                skippedCount = skippedCount,
                                failedCount = failedCount,
                                fileSize = fileSize,
                                isRetry = true
                            )
                        } else {
                            Log.e(TAG, "Hash verification failed but cannot retry (isRetry=$isRetry, fileSize=$fileSize)")
                        }
                    }
                } catch (e: Exception) {
                    Log.e(TAG, "Failed to parse 400 error response for $fileName", e)
                }
            } else {
                val responseBody = response.body?.string()
                Log.e(TAG, "Upload failed: ${response.code} - $responseBody for $fileName")
                LogCollector.e(TAG, "Upload FAILED (${response.code}): $fileName - $responseBody")
            }
        } catch (e: java.io.IOException) {
            // This exception is thrown when the call is cancelled
            if (isStopped) {
                Log.d(TAG, "Upload cancelled during execution: $fileName")
                LogCollector.i(TAG, "Upload cancelled: $fileName")
            } else {
                Log.e(TAG, "Upload failed with IO error for $fileName", e)
                LogCollector.e(TAG, "Upload IO ERROR: $fileName - ${e.message}")
            }
            activeCall = null
        } catch (e: Exception) {
            Log.e(TAG, "Upload failed with error for $fileName", e)
            LogCollector.e(TAG, "Upload EXCEPTION: $fileName - ${e.javaClass.simpleName}: ${e.message}")
            activeCall = null
        }

        return false
    }
}