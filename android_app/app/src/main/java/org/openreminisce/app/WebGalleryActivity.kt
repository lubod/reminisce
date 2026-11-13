package org.openreminisce.app

import android.annotation.SuppressLint
import android.net.http.SslError
import android.os.Bundle
import android.util.Log
import android.view.MenuItem
import android.view.View
import android.webkit.*
import android.widget.ProgressBar
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import org.json.JSONObject
import org.openreminisce.app.service.BackupController
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.AuthenticatedHttpClient
import org.openreminisce.app.util.CertificateMismatchException
import org.openreminisce.app.util.DatabaseHelper
import org.openreminisce.app.util.NetworkHelper
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.util.SecureStorageHelper
import org.openreminisce.app.util.TofuPinningSupport

class WebGalleryActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "WebGalleryActivity"
    }

    private lateinit var webView: WebView
    private lateinit var progressBar: ProgressBar
    private lateinit var databaseHelper: DatabaseHelper
    private lateinit var backupController: BackupController
    @Volatile
    private var authToken: String? = null
    @Volatile
    private var currentPageUrl: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_web_gallery)

        // Enable back button in action bar
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        supportActionBar?.title = "Web Gallery"

        webView = findViewById(R.id.webView)
        progressBar = findViewById(R.id.webProgressBar)

        databaseHelper = DatabaseHelper(this)
        backupController = BackupController(this, databaseHelper)
        backupController.attach()

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

    private fun startQuickBackup() = backupController.startQuickBackup()

    private fun startFullBackup() = backupController.startFullBackup()

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

            // Enable mixed content only for local/private networks
            val serverUrl = PreferenceHelper.getServerUrl(this@WebGalleryActivity)
            mixedContentMode = if (NetworkHelper.isPrivateOrLocalAddress(serverUrl)) {
                WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
            } else {
                WebSettings.MIXED_CONTENT_NEVER_ALLOW
            }

            // File access is disabled for security (assets load fine regardless)
            allowFileAccess = false
            allowContentAccess = false

            // Additional settings for better compatibility and caching
            loadWithOverviewMode = true
            useWideViewPort = true

            // Enable safe browsing
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
                safeBrowsingEnabled = true
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
                currentPageUrl = url
                Log.d(TAG, "Page started loading: $url")
                progressBar.visibility = View.VISIBLE
            }

            override fun onPageFinished(view: WebView?, url: String?) {
                super.onPageFinished(view, url)
                currentPageUrl = url
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
                val url = error?.url ?: currentPageUrl ?: ""
                if (!NetworkHelper.isPrivateOrLocalAddress(url)) {
                    Log.e(TAG, "SSL error ${error?.primaryError} on public domain: cancelling")
                    handler?.cancel()
                    return
                }

                val x509 = TofuPinningSupport.extractX509Certificate(error?.certificate)
                val hostPort = TofuPinningSupport.hostPortKey(url)
                if (x509 == null || hostPort == null) {
                    Log.e(TAG, "Unable to inspect leaf certificate for $url; cancelling")
                    handler?.cancel()
                    return
                }

                val actualSha256 = TofuPinningSupport.certificateSha256Hex(x509)
                val pinnedSha256 = SecureStorageHelper.getCertPin(this@WebGalleryActivity, hostPort)
                when {
                    pinnedSha256 == null -> {
                        // First time connecting to this private/local self-signed cert: TOFU pin.
                        SecureStorageHelper.setCertPin(this@WebGalleryActivity, hostPort, actualSha256)
                        handler?.proceed()
                    }
                    pinnedSha256.equals(actualSha256, ignoreCase = true) -> {
                        handler?.proceed()
                    }
                    else -> {
                        Log.w(TAG, "TLS certificate mismatch in WebView for $hostPort")
                        if (isFinishing || isDestroyed) {
                            handler?.cancel()
                            return
                        }
                        val mismatch = CertificateMismatchException(
                            hostPort = hostPort,
                            expectedSha256 = pinnedSha256,
                            actualSha256 = actualSha256
                        )
                        TofuPinningSupport.showMismatchDialog(
                            activity = this@WebGalleryActivity,
                            mismatch = mismatch,
                            onTrustNewCertificate = {
                                SecureStorageHelper.setCertPin(this@WebGalleryActivity, hostPort, actualSha256)
                                AuthenticatedHttpClient.invalidateClient()
                                handler?.proceed()
                            },
                            onCancel = { handler?.cancel() }
                        )
                    }
                }
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
        currentPageUrl = null
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

    private fun sameOrigin(a: String, b: String): Boolean {
        return try {
            val u1 = java.net.URL(a)
            val u2 = java.net.URL(b)
            val p1 = if (u1.port != -1) u1.port else u1.defaultPort
            val p2 = if (u2.port != -1) u2.port else u2.defaultPort
            u1.protocol.equals(u2.protocol, ignoreCase = true) &&
                u1.host.equals(u2.host, ignoreCase = true) &&
                p1 == p2
        } catch (e: Exception) {
            false
        }
    }

    private fun injectAuthToken() {
        // Inject the auth token only into pages served by the configured server
        // origin. A redirect, compromised gallery page, or injected iframe must
        // never receive the JWT (previously it was written to localStorage on
        // every onPageFinished regardless of origin).
        val currentUrl = currentPageUrl
        val configuredUrl = PreferenceHelper.getServerUrl(this)
        if (currentUrl == null || !sameOrigin(currentUrl, configuredUrl)) {
            Log.w(TAG, "Skipping auth token injection: origin mismatch (url=$currentUrl server=$configuredUrl)")
            return
        }
        // Inject the auth token into localStorage using JSONObject.quote for safe JS string escaping
        authToken?.let { token ->
            val deviceId = AuthHelper.getDeviceId(this)
            val quotedToken = JSONObject.quote(token)
            val quotedDeviceId = JSONObject.quote(deviceId)
            val javascript = """
                (function() {
                    try {
                        localStorage.setItem('authToken', $quotedToken);
                        localStorage.setItem('deviceId', $quotedDeviceId);
                        console.log('Auth token injected successfully');

                        // Dispatch a custom event to notify the React app
                        window.dispatchEvent(new CustomEvent('authTokenReady', {
                            detail: { token: $quotedToken, deviceId: $quotedDeviceId }
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
                if (!backupController.isBackingUp) {
                    startQuickBackup()
                }
                true
            }
            R.id.action_full_backup -> {
                if (!backupController.isBackingUp) {
                    startFullBackup()
                }
                true
            }
            else -> super.onOptionsItemSelected(item)
        }
    }

    override fun onDestroy() {
        backupController.onDestroy()
        super.onDestroy()
        webView.destroy()
    }

    // JavaScript interface for communication between WebView and Android
    inner class WebAppInterface {
        private fun isTrustedOrigin(): Boolean {
            val url = currentPageUrl ?: return false
            val configuredUrl = PreferenceHelper.getServerUrl(this@WebGalleryActivity)
            return sameOrigin(url, configuredUrl)
        }

        @JavascriptInterface
        fun getAuthToken(): String {
            if (!isTrustedOrigin()) {
                Log.w(TAG, "Blocked getAuthToken from untrusted origin: $currentPageUrl")
                return ""
            }
            return authToken ?: ""
        }

        @JavascriptInterface
        fun getDeviceId(): String {
            if (!isTrustedOrigin()) {
                Log.w(TAG, "Blocked getDeviceId from untrusted origin: $currentPageUrl")
                return ""
            }
            return AuthHelper.getDeviceId(this@WebGalleryActivity)
        }

        @JavascriptInterface
        fun getServerUrl(): String {
            if (!isTrustedOrigin()) {
                Log.w(TAG, "Blocked getServerUrl from untrusted origin: $currentPageUrl")
                return ""
            }
            return PreferenceHelper.getServerUrl(this@WebGalleryActivity)
        }

        @JavascriptInterface
        fun refreshToken(): String {
            if (!isTrustedOrigin()) {
                Log.w(TAG, "Blocked refreshToken from untrusted origin: $currentPageUrl")
                return ""
            }
            return try {
                val refreshed = AuthHelper.getValidToken(this@WebGalleryActivity) ?: ""
                if (refreshed.isNotEmpty()) {
                    authToken = refreshed
                }
                refreshed
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