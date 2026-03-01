package org.openreminisce.app

import android.annotation.SuppressLint
import android.content.Intent
import android.net.http.SslError
import android.os.Bundle
import android.util.Log
import android.view.MenuItem
import android.view.View
import android.webkit.*
import android.widget.Button
import android.widget.ProgressBar
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.work.WorkManager
import org.openreminisce.app.service.BackupService
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.BackupProgressDialogHelper
import org.openreminisce.app.util.DatabaseHelper
import org.openreminisce.app.util.NetworkHelper
import org.openreminisce.app.util.PreferenceHelper
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class WebGalleryActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "WebGalleryActivity"
    }

    private lateinit var webView: WebView
    private lateinit var progressBar: ProgressBar
    private lateinit var databaseHelper: DatabaseHelper
    private lateinit var progressDialogHelper: BackupProgressDialogHelper
    private var authToken: String? = null
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
        setContentView(R.layout.activity_web_gallery)

        // Enable back button in action bar
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        supportActionBar?.title = "Web Gallery"

        webView = findViewById(R.id.webView)
        progressBar = findViewById(R.id.webProgressBar)

        databaseHelper = DatabaseHelper(this)
        progressDialogHelper = BackupProgressDialogHelper(this)

        // Register broadcast receiver for backup status and progress updates
        val filter = android.content.IntentFilter("org.openreminisce.app.BACKUP_STATUS")
        filter.addAction("org.openreminisce.app.BACKUP_PROGRESS")

        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(backupStatusReceiver, filter, android.content.Context.RECEIVER_NOT_EXPORTED)
        } else {
            androidx.core.content.ContextCompat.registerReceiver(this, backupStatusReceiver, filter, androidx.core.content.ContextCompat.RECEIVER_NOT_EXPORTED)
        }

        // Handle back button press for WebView navigation
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (webView.canGoBack()) {
                    webView.goBack()
                } else {
                    isEnabled = false
                    onBackPressedDispatcher.onBackPressed()
                }
            }
        })

        setupWebView()
        loadGallery()
    }

    private fun startQuickBackup() {
        isBackingUp = true

        // Clear any previous cancel flag
        val prefs = getSharedPreferences("BackupState", MODE_PRIVATE)
        prefs.edit().putBoolean("cancel_backup", false).apply()

        // Get the last backup timestamp
        val lastBackupTimestamp = databaseHelper.getLastImageBackupTimestamp()

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

        // Start backup service with intent - default to images
        val intent = Intent(this, BackupService::class.java).apply {
            putExtra("backup_type", "image")
            putExtra("quick_backup", true)
        }
        startService(intent)
    }

    private fun startFullBackup() {
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

        // Start backup service with intent - default to images
        val intent = Intent(this, BackupService::class.java).apply {
            putExtra("backup_type", "image")
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
                // On dismiss - nothing special needed for WebGalleryActivity
            }
        }
    }

    private fun handleBackupFailedOrCancelled(status: String?) {
        runOnUiThread {
            progressDialogHelper.showFailure(status ?: "failed") {
                // On dismiss - nothing special needed
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

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWebView() {
        // Check network connectivity and set appropriate cache mode
        val isOnline = NetworkHelper.isNetworkAvailable(this)
        val cacheMode = if (isOnline) {
            WebSettings.LOAD_DEFAULT  // Load from network, use cache for already cached resources
        } else {
            WebSettings.LOAD_CACHE_ELSE_NETWORK  // Use cache if available, otherwise try network
        }

        Log.d(TAG, "Network status: ${if (isOnline) "ONLINE" else "OFFLINE"}, Cache mode: $cacheMode")

        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            this.cacheMode = cacheMode

            // Enable zooming
            setSupportZoom(true)
            builtInZoomControls = true
            displayZoomControls = false

            // Enable mixed content for local development
            mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW

            // Allow file access for offline page
            allowFileAccess = true
            allowContentAccess = true

            // Additional settings for better compatibility and caching
            loadWithOverviewMode = true
            useWideViewPort = true

            // Enable safe browsing disabled for development
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
                safeBrowsingEnabled = false
            }
        }

        // Set WebViewClient to handle page navigation
        webView.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView?, request: WebResourceRequest?): Boolean {
                Log.d(TAG, "Loading URL: ${request?.url}")
                // Allow all URLs to load in the WebView
                return false
            }

            override fun onPageStarted(view: WebView?, url: String?, favicon: android.graphics.Bitmap?) {
                super.onPageStarted(view, url, favicon)
                Log.d(TAG, "Page started loading: $url")
                progressBar.visibility = View.VISIBLE
            }

            override fun onPageFinished(view: WebView?, url: String?) {
                super.onPageFinished(view, url)
                Log.d(TAG, "Page finished loading: $url")
                progressBar.visibility = View.GONE

                // Inject auth token into the page after it loads
                injectAuthToken()
            }

            override fun onReceivedError(
                view: WebView?,
                request: WebResourceRequest?,
                error: WebResourceError?
            ) {
                super.onReceivedError(view, request, error)

                val errorCode = if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.M) {
                    error?.errorCode ?: WebViewClient.ERROR_UNKNOWN
                } else {
                    WebViewClient.ERROR_UNKNOWN
                }

                val errorDescription = if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.M) {
                    error?.description?.toString() ?: "Unknown error"
                } else {
                    "Unknown error"
                }

                Log.e(TAG, "WebView error for ${request?.url}: $errorDescription (code: $errorCode)")

                // Only handle errors for main frame (not for resources like images, scripts, etc.)
                if (request?.isForMainFrame == true) {
                    progressBar.visibility = View.GONE
                    loadErrorPage("Connection Error ($errorCode)")
                }
            }

            override fun onReceivedHttpError(
                view: WebView?,
                request: WebResourceRequest?,
                errorResponse: WebResourceResponse?
            ) {
                super.onReceivedHttpError(view, request, errorResponse)
                val statusCode = errorResponse?.statusCode ?: 0
                Log.e(TAG, "HTTP error for ${request?.url}: $statusCode - ${errorResponse?.reasonPhrase}")
                
                if (request?.isForMainFrame == true) {
                    progressBar.visibility = View.GONE
                    loadErrorPage("Server Error ($statusCode)")
                }
            }

            override fun onReceivedSslError(
                view: WebView?,
                handler: SslErrorHandler?,
                error: SslError?
            ) {
                // For development with self-signed certificates
                // WARNING: In production, you should properly validate SSL certificates
                Log.w(TAG, "SSL error: ${error?.primaryError}, proceeding anyway for development")
                handler?.proceed()
            }

            override fun onRenderProcessGone(view: WebView?, detail: android.webkit.RenderProcessGoneDetail?): Boolean {
                if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
                    Log.e(TAG, "Renderer process crashed! Did crash: ${detail?.didCrash()}, Priority: ${detail?.rendererPriorityAtExit()}")
                } else {
                    Log.e(TAG, "Renderer process crashed!")
                }

                runOnUiThread {
                    finish()
                }
                return true
            }
        }

        // Set WebChromeClient for better JavaScript support
        webView.webChromeClient = object : WebChromeClient() {
            override fun onProgressChanged(view: WebView?, newProgress: Int) {
                super.onProgressChanged(view, newProgress)
                progressBar.progress = newProgress
            }

            override fun onConsoleMessage(consoleMessage: ConsoleMessage?): Boolean {
                Log.d(TAG, "Console: ${consoleMessage?.message()} -- Line ${consoleMessage?.lineNumber()} of ${consoleMessage?.sourceId()}")
                return true
            }
        }

        // Add JavaScript interface for bidirectional communication
        webView.addJavascriptInterface(WebAppInterface(), "AndroidBridge")
    }

    private fun loadGallery() {
        // Get server URL from preferences
        val serverUrl = PreferenceHelper.getServerUrl(this)

        if (serverUrl.isEmpty()) {
            Log.e(TAG, "No server URL configured")
            finish()
            return
        }

        // Get authentication token
        Thread {
            try {
                authToken = AuthHelper.getValidToken(this)

                if (authToken.isNullOrEmpty()) {
                    runOnUiThread {
                        Log.e(TAG, "Failed to get auth token")
                        finish()
                    }
                    return@Thread
                }

                // Load the React app
                runOnUiThread {
                    // Test different possible paths where React app might be hosted
                    val possibleUrls = listOf(
                        serverUrl,                    // Root
                        "$serverUrl/",                // Root with trailing slash
                        "$serverUrl/gallery",         // /gallery path
                        "$serverUrl/web",             // /web path
                        "$serverUrl/index.html"       // index.html directly
                    )

                    // For now, try the root URL
                    // TODO: If this doesn't work, user can modify to try other paths
                    val urlToLoad = serverUrl

                    Log.d(TAG, "Loading gallery URL: $urlToLoad")
                    Log.d(TAG, "Available URL options if root doesn't work: ${possibleUrls.joinToString()}")

                    webView.loadUrl(urlToLoad)
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error loading gallery", e)
            }
        }.start()
    }

    private fun loadErrorPage(message: String) {
        val errorHtml = """
            <!DOCTYPE html>
            <html>
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <style>
                    body { background-color: black; color: white; display: flex; flex-direction: column; justify-content: center; align-items: center; height: 100vh; margin: 0; font-family: sans-serif; }
                    button { background-color: #333; color: white; border: 1px solid #555; padding: 12px 24px; font-size: 16px; border-radius: 4px; cursor: pointer; }
                    button:active { background-color: #444; }
                    .error-info { color: #666; margin-bottom: 20px; font-size: 14px; }
                </style>
            </head>
            <body>
                <div class="error-info">$message</div>
                <button onclick="AndroidBridge.reloadGallery()">Refresh</button>
            </body>
            </html>
        """.trimIndent()
        webView.loadDataWithBaseURL(null, errorHtml, "text/html", "UTF-8", null)
    }

    private fun injectAuthToken() {
        // Inject the auth token into localStorage so the React app can use it
        authToken?.let { token ->
            val deviceId = AuthHelper.getDeviceId(this)
            val javascript = """
                (function() {
                    try {
                        localStorage.setItem('authToken', '$token');
                        localStorage.setItem('deviceId', '$deviceId');
                        console.log('Auth token injected successfully');

                        // Dispatch a custom event to notify the React app
                        window.dispatchEvent(new CustomEvent('authTokenReady', {
                            detail: { token: '$token', deviceId: '$deviceId' }
                        }));
                    } catch (e) {
                        console.error('Failed to inject auth token:', e);
                    }
                })();
            """.trimIndent()

            webView.evaluateJavascript(javascript) { result ->
                Log.d(TAG, "Token injection result: $result")
            }
        }
    }

    override fun onCreateOptionsMenu(menu: android.view.Menu): Boolean {
        menuInflater.inflate(R.menu.web_gallery_menu, menu)
        return true
    }

    override fun onOptionsItemSelected(item: MenuItem): Boolean {
        return when (item.itemId) {
            android.R.id.home -> {
                finish()
                true
            }
            R.id.action_quick_backup -> {
                if (!isBackingUp) {
                    startQuickBackup()
                }
                true
            }
            R.id.action_full_backup -> {
                if (!isBackingUp) {
                    startFullBackup()
                }
                true
            }
            else -> super.onOptionsItemSelected(item)
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        try {
            unregisterReceiver(backupStatusReceiver)
        } catch (e: Exception) {
            // Receiver was not registered or already unregistered
        }
        progressDialogHelper.dismiss()
        webView.destroy()
    }

    // JavaScript interface for communication between WebView and Android
    inner class WebAppInterface {
        @JavascriptInterface
        fun getAuthToken(): String {
            return authToken ?: ""
        }

        @JavascriptInterface
        fun getDeviceId(): String {
            return AuthHelper.getDeviceId(this@WebGalleryActivity)
        }

        @JavascriptInterface
        fun getServerUrl(): String {
            return PreferenceHelper.getServerUrl(this@WebGalleryActivity)
        }

        @JavascriptInterface
        fun refreshToken(): String {
            // This will be called from JavaScript if token expires
            return try {
                AuthHelper.getValidToken(this@WebGalleryActivity) ?: ""
            } catch (e: Exception) {
                Log.e(TAG, "Error refreshing token", e)
                ""
            }
        }

        @JavascriptInterface
        fun showToast(message: String) {
            Log.d(TAG, "JS Toast: $message")
        }

        @JavascriptInterface
        fun isNetworkAvailable(): Boolean {
            return NetworkHelper.isNetworkAvailable(this@WebGalleryActivity)
        }

        @JavascriptInterface
        fun reloadGallery() {
            // Called from offline page to reload the main gallery
            runOnUiThread {
                if (NetworkHelper.isNetworkAvailable(this@WebGalleryActivity)) {
                    loadGallery()
                } else {
                    showToast("Still offline. Please check your connection.")
                }
            }
        }
    }
}