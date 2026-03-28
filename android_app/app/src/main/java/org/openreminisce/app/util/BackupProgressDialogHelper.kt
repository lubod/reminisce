package org.openreminisce.app.util

import android.app.AlertDialog
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.LayoutInflater
import android.view.View
import android.widget.Button
import android.widget.ProgressBar
import android.widget.TextView
import org.openreminisce.app.R

class BackupProgressDialogHelper(private val context: Context) {
    companion object {
        private const val TAG = "BackupProgressDialog"
        private const val PROGRESS_UPDATE_THROTTLE_MS = 300L
    }

    private var progressDialog: AlertDialog? = null
    private var dialogView: View? = null
    private var progressBar: ProgressBar? = null
    private var fileProgressBar: ProgressBar? = null
    private var progressText: TextView? = null
    private var cancelButton: Button? = null
    private var progressDialogTitle: TextView? = null
    private var statsTotalFiles: TextView? = null
    private var statsUploaded: TextView? = null
    private var statsSkipped: TextView? = null
    private var statsFailed: TextView? = null
    private var backupContextMessage: String? = null
    private var pendingProgressUpdate: Triple<Float, String, String>? = null
    private var lastProgressUpdateTimestamp = 0L
    private var onCancelCallback: (() -> Unit)? = null

    fun show(initialMessage: String = "Starting upload...", isQuickBackup: Boolean = false, quickBackupTimestampText: String? = null, onCancel: () -> Unit) {
        onCancelCallback = onCancel

        // If dialog is already showing, just update the message and return
        if (progressDialog?.isShowing == true) {
            Log.d(TAG, "Progress dialog already showing, updating message only")
            progressText?.text = initialMessage
            return
        }

        // Dismiss any existing dialog
        progressDialog?.dismiss()
        clearReferences()

        // Inflate the dialog layout
        val inflater = LayoutInflater.from(context)
        val inflatedView = inflater.inflate(R.layout.dialog_backup_progress, null)
        dialogView = inflatedView

        // Initialize UI elements from the inflated view
        progressBar = inflatedView.findViewById(R.id.progressBar)
        fileProgressBar = inflatedView.findViewById(R.id.fileProgressBar)
        progressText = inflatedView.findViewById(R.id.progressText)
        cancelButton = inflatedView.findViewById(R.id.cancelButton)
        progressDialogTitle = inflatedView.findViewById(R.id.progressDialogTitle)
        statsTotalFiles = inflatedView.findViewById(R.id.statsTotalFiles)
        statsUploaded = inflatedView.findViewById(R.id.statsUploaded)
        statsSkipped = inflatedView.findViewById(R.id.statsSkipped)
        statsFailed = inflatedView.findViewById(R.id.statsFailed)

        // Set initial values
        progressText?.text = initialMessage
        if (backupContextMessage.isNullOrEmpty()) {
            backupContextMessage = initialMessage
        }
        progressBar?.progress = 0
        progressBar?.max = 100
        fileProgressBar?.progress = 0
        fileProgressBar?.max = 100

        // Set title based on backup type
        val dialogTitleText = if (isQuickBackup) {
            if (quickBackupTimestampText != null && quickBackupTimestampText.isNotEmpty()) {
                "${context.getString(R.string.quick_backup)} $quickBackupTimestampText"
            } else {
                context.getString(R.string.quick_backup)
            }
        } else {
            context.getString(R.string.backup_all)
        }
        progressDialogTitle?.text = dialogTitleText
        progressBar?.visibility = View.VISIBLE
        fileProgressBar?.visibility = View.VISIBLE

        // Configure the dialog
        val builder = AlertDialog.Builder(context)
            .setView(inflatedView)
            .setCancelable(false)

        val dialog = builder.create()

        // Set cancel button click listener
        cancelButton?.setOnClickListener {
            onCancelCallback?.invoke()
            dismiss()
        }

        // Store reference to dialog
        progressDialog = dialog

        // Show the dialog
        dialog.show()

        // Apply any pending progress update if available
        if (pendingProgressUpdate != null) {
            Handler(Looper.getMainLooper()).post {
                val (overallProgress, currentAction, currentFile) = pendingProgressUpdate!!
                var progressValue = overallProgress
                if (progressValue.isNaN() || progressValue.isInfinite()) {
                    progressValue = 0f
                }
                val clampedProgress = (progressValue * 100).toInt().coerceIn(0, 100)
                progressBar?.progress = clampedProgress
                progressText?.text = "$currentAction: $currentFile"
                pendingProgressUpdate = null
            }
        }

        Log.d(TAG, "Progress dialog shown and ready for updates")
    }

    fun updateProgress(
        overallProgress: Float,
        currentAction: String,
        currentFile: String,
        fileIndex: Int,
        totalFiles: Int,
        backedUpCount: Int = 0,
        skippedCount: Int = 0,
        failedCount: Int = 0,
        fileProgress: Float = 0f,
        fileUploadProgress: Float = 0f
    ) {
        // Throttle UI updates
        val currentTime = System.currentTimeMillis()
        if (currentTime - lastProgressUpdateTimestamp < PROGRESS_UPDATE_THROTTLE_MS) {
            pendingProgressUpdate = Triple(overallProgress, currentAction, currentFile)
            return
        }
        lastProgressUpdateTimestamp = currentTime

        // Update progress bar
        val actionText = when (currentAction) {
            "scanning" -> context.getString(R.string.status_scanning)
            "analyzing" -> context.getString(R.string.status_preparing)
            "calculating_hash" -> context.getString(R.string.status_calculating_hash, currentFile)
            "checking_server" -> {
                // currentFile now contains "X/Y files" from the worker
                context.getString(R.string.status_checking_server, currentFile)
            }
            "uploading" -> {
                 if (fileUploadProgress > 0f) {
                     val percent = (fileUploadProgress * 100).toInt()
                     context.getString(R.string.status_uploading_percentage, currentFile, percent.toString())
                 } else {
                     context.getString(R.string.status_uploading, currentFile)
                 }
            }
            "uploaded" -> context.getString(R.string.status_upload_success, currentFile)
            "upload_failed" -> context.getString(R.string.status_upload_failed, currentFile)
            "skipped_existing_file", "skipped" -> context.getString(R.string.status_skipped, currentFile)
            "completed_no_files" -> context.getString(R.string.status_completed_no_files)
            "pre_scan_complete" -> context.getString(R.string.status_pre_scan_complete)
            else -> context.getString(R.string.processing_files)
        }

        // We don't need to append filename again if it's already in the action text
        val progressMessage = buildString {
            append(actionText).append("\n")
            append("Overall Progress: $fileIndex/$totalFiles")
        }

        if (progressDialog?.isShowing == true && progressBar != null && progressText != null) {
            var progressValue = overallProgress
            if (progressValue.isNaN() || progressValue.isInfinite()) {
                progressValue = 0f
            }
            val clampedProgress = (progressValue * 100).toInt().coerceIn(0, 100)
            progressBar?.progress = clampedProgress
            progressText?.text = progressMessage

            // Update statistics
            statsTotalFiles?.text = totalFiles.toString()
            statsUploaded?.text = backedUpCount.toString()
            statsSkipped?.text = skippedCount.toString()
            statsFailed?.text = failedCount.toString()

            // Update file progress bar
            val currentFileProgress = if (fileUploadProgress > 0f) fileUploadProgress else fileProgress
            var fileProgressValue = currentFileProgress
            if (fileProgressValue.isNaN() || fileProgressValue.isInfinite()) {
                fileProgressValue = 0f
            }
            val clampedFileProgress = (fileProgressValue * 100).toInt().coerceIn(0, 100)
            fileProgressBar?.progress = clampedFileProgress

            progressBar?.invalidate()
            fileProgressBar?.invalidate()
        } else {
            pendingProgressUpdate = Triple(overallProgress, actionText, progressMessage)
        }
    }

    fun showCompletion(
        successfullyBackedUp: Int,
        totalProcessed: Int,
        skippedExisting: Int,
        failedCount: Int,
        backupType: String,
        isQuickBackup: Boolean,
        onDismiss: () -> Unit
    ) {
        val mediaTypeText = when (backupType) {
            "video" -> context.getString(R.string.videos)
            "all" -> "media files"
            else -> context.getString(R.string.photos)
        }

        val message = when {
            failedCount > 0 && successfullyBackedUp > 0 -> {
                "Upload completed with errors:\n" +
                        "- Uploaded: $successfullyBackedUp\n" +
                        "- Skipped (existing): $skippedExisting\n" +
                        "- Failed: $failedCount\n" +
                        "Total processed: $totalProcessed"
            }
            failedCount > 0 && successfullyBackedUp == 0 -> {
                "Upload failed:\n" +
                        "- Failed: $failedCount of $totalProcessed files\n" +
                        "Please check your connection and try again."
            }
            successfullyBackedUp == 0 && skippedExisting == totalProcessed -> {
                "All files already uploaded!\n" +
                        "- Total checked: $totalProcessed\n" +
                        "- Already on server: $skippedExisting"
            }
            successfullyBackedUp > 0 && skippedExisting > 0 -> {
                "Upload completed successfully!\n" +
                        "- Uploaded: $successfullyBackedUp\n" +
                        "- Skipped (existing): $skippedExisting\n" +
                        "Total processed: $totalProcessed"
            }
            successfullyBackedUp > 0 -> {
                "Upload completed successfully!\n" +
                        "- Uploaded: $successfullyBackedUp of $totalProcessed files"
            }
            else -> {
                "Upload operation completed.\n" +
                        "Processed: $totalProcessed $mediaTypeText"
            }
        }

        if (progressDialog?.isShowing == true) {
            progressText?.text = message
            progressBar?.progress = 100

            // Update final statistics
            statsTotalFiles?.text = totalProcessed.toString()
            statsUploaded?.text = successfullyBackedUp.toString()
            statsSkipped?.text = skippedExisting.toString()
            statsFailed?.text = failedCount.toString()

            val backupTypeText = if (isQuickBackup && !backupContextMessage.isNullOrEmpty()) {
                "Finished quick $backupContextMessage"
            } else if (isQuickBackup) {
                "Finished ${context.getString(R.string.quick_backup)}"
            } else {
                context.getString(R.string.backup_completed)
            }
            progressDialogTitle?.text = backupTypeText

            // Change cancel button to OK button
            cancelButton?.text = context.getString(R.string.ok)
            cancelButton?.setOnClickListener {
                backupContextMessage = null
                dismiss()
                onDismiss()
            }
        }
    }

    fun showFailure(status: String, onDismiss: () -> Unit) {
        val message = if (status == "failed") "Upload failed" else "Upload cancelled"

        if (progressDialog?.isShowing == true) {
            progressText?.text = message
            progressBar?.progress = 100
            progressDialogTitle?.text = if (status == "failed") {
                context.getString(R.string.backup_failed)
            } else {
                context.getString(R.string.backup_cancelled_title)
            }

            cancelButton?.text = context.getString(R.string.ok)
            cancelButton?.setOnClickListener {
                backupContextMessage = null
                dismiss()
                onDismiss()
            }
        }
    }

    fun dismiss() {
        progressDialog?.dismiss()
        clearReferences()
    }

    fun isShowing(): Boolean = progressDialog?.isShowing == true

    private fun clearReferences() {
        progressDialog = null
        dialogView = null
        progressBar = null
        fileProgressBar = null
        progressText = null
        cancelButton = null
        progressDialogTitle = null
        statsTotalFiles = null
        statsUploaded = null
        statsSkipped = null
        statsFailed = null
    }
}
