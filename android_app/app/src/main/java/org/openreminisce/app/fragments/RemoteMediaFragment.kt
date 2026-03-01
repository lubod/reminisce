package org.openreminisce.app.fragments

import android.annotation.SuppressLint
import android.content.Intent
import android.net.http.SslError
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.webkit.*
import android.widget.ProgressBar
import androidx.core.view.MenuProvider
import androidx.fragment.app.Fragment
import androidx.lifecycle.Lifecycle
import org.openreminisce.app.LoginActivity
import org.openreminisce.app.R
import org.openreminisce.app.service.BackupService
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.NetworkHelper
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.util.SecureStorageHelper

class RemoteMediaFragment : Fragment() {

    companion object {
        private const val TAG = "RemoteMediaFragment"
    }

    private lateinit var webView: WebView
    private var progressBar: ProgressBar? = null
    private var authToken: String? = null

    override fun onCreateView(
        inflater: LayoutInflater,
        container: ViewGroup?,
        savedInstanceState: Bundle?
    ): View? {
        return inflater.inflate(R.layout.fragment_remote_media, container, false)
    }

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        webView = view.findViewById(R.id.webView)
        progressBar = view.findViewById(R.id.webProgressBar)

        setupWebView()
        setupMenu()
        loadGallery()
    }

    private fun setupMenu() {
        requireActivity().addMenuProvider(object : MenuProvider {
            override fun onCreateMenu(menu: android.view.Menu, menuInflater: android.view.MenuInflater) {
                // Menu is already inflated in MainActivity
            }

            override fun onMenuItemSelected(menuItem: android.view.MenuItem): Boolean {
                return when (menuItem.itemId) {
                    R.id.action_refresh -> {
                        webView.reload()
                        true
                    }
                    else -> false
                }
            }
        }, viewLifecycleOwner, Lifecycle.State.RESUMED)
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWebView() {
        val context = requireContext()
        // Check network connectivity and set appropriate cache mode
        val isOnline = NetworkHelper.isNetworkAvailable(context)
        val cacheMode = if (isOnline) {
            WebSettings.LOAD_DEFAULT
        } else {
            WebSettings.LOAD_CACHE_ELSE_NETWORK
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
                return false
            }

            override fun onPageStarted(view: WebView?, url: String?, favicon: android.graphics.Bitmap?) {
                super.onPageStarted(view, url, favicon)
                Log.d(TAG, "Page started loading: $url")
                progressBar?.visibility = View.VISIBLE
            }

            override fun onPageFinished(view: WebView?, url: String?) {
                super.onPageFinished(view, url)
                Log.d(TAG, "Page finished loading: $url")
                progressBar?.visibility = View.GONE

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

                // Only handle errors for main frame
                if (request?.isForMainFrame == true) {
                    progressBar?.visibility = View.GONE
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
                    progressBar?.visibility = View.GONE
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

                activity?.runOnUiThread {
                    activity?.finish()
                }
                return true
            }
        }

        // Set WebChromeClient for better JavaScript support
        webView.webChromeClient = object : WebChromeClient() {
            override fun onProgressChanged(view: WebView?, newProgress: Int) {
                super.onProgressChanged(view, newProgress)
                progressBar?.progress = newProgress
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
        val context = context ?: return
        // Get server URL from preferences
        val serverUrl = PreferenceHelper.getServerUrl(context)

        if (serverUrl.isEmpty()) {
            Log.e(TAG, "No server URL configured")
            return
        }

        // Get authentication token in background, then load the URL
        Thread {
            try {
                // Get the token synchronously in this background thread
                val token = AuthHelper.getValidToken(context)

                Log.d(TAG, "Retrieved auth token: ${if (token.isNullOrEmpty()) "NULL/EMPTY" else "SUCCESS (${token.length} chars)"}")

                if (token.isNullOrEmpty()) {
                    activity?.runOnUiThread {
                        Log.e(TAG, "Failed to get auth token, redirecting to login")
                        logout()
                    }
                    return@Thread
                }

                // Store the token BEFORE loading the WebView
                authToken = token
                Log.d(TAG, "authToken variable set, length: ${authToken?.length}")

                // Inject token BEFORE loading the page by using a data URL that sets localStorage first
                activity?.runOnUiThread {
                    Log.d(TAG, "Pre-injecting token via data URL before loading: $serverUrl")
                    val deviceId = AuthHelper.getDeviceId(context)

                    // Create an HTML page that sets localStorage and then redirects
                    val htmlContent = """
                        <!DOCTYPE html>
                        <html>
                        <head>
                            <meta charset="utf-8">
                            <title>Loading...</title>
                        </head>
                        <body>
                            <div style="display: flex; justify-content: center; align-items: center; height: 100vh; font-family: Arial;">
                                <div>Loading...</div>
                            </div>
                            <script>
                                (function() {
                                    try {
                                        // Set the auth token and device ID in localStorage
                                        localStorage.setItem('authToken', '$token');
                                        localStorage.setItem('deviceId', '$deviceId');
                                        console.log('Auth token pre-injected via data URL');

                                        // Dispatch event so web app knows token is ready
                                        window.dispatchEvent(new CustomEvent('authTokenReady', {
                                            detail: { token: '$token', deviceId: '$deviceId' }
                                        }));

                                        // Redirect to the actual page
                                        setTimeout(function() {
                                            window.location.href = '$serverUrl/media?hidemenu=true';
                                        }, 100);
                                    } catch (e) {
                                        console.error('Failed to pre-inject auth token:', e);
                                        // Redirect anyway
                                        window.location.href = '$serverUrl/media?hidemenu=true';
                                    }
                                })();
                            </script>
                        </body>
                        </html>
                    """.trimIndent()

                    webView.loadDataWithBaseURL(serverUrl, htmlContent, "text/html", "UTF-8", null)
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
        // Inject the auth token into localStorage so the web app can use it
        Log.d(TAG, "injectAuthToken() called, authToken is ${if (authToken == null) "NULL" else "available (${authToken!!.length} chars)"}")

        authToken?.let { token ->
            val context = context ?: return
            val deviceId = AuthHelper.getDeviceId(context)
            Log.d(TAG, "Injecting token into WebView localStorage...")

            val javascript = """
                (function() {
                    try {
                        localStorage.setItem('authToken', '$token');
                        localStorage.setItem('deviceId', '$deviceId');
                        console.log('Auth token injected successfully');

                        // Dispatch a custom event to notify the web app
                        window.dispatchEvent(new CustomEvent('authTokenReady', {
                            detail: { token: '$token', deviceId: '$deviceId' }
                        }));
                    } catch (e) {
                        console.error('Failed to inject auth token:', e);
                    }
                })();
            """.trimIndent()

            webView.evaluateJavascript(javascript) { result ->
                Log.d(TAG, "Token injection completed, result: $result")
            }
        } ?: run {
            Log.e(TAG, "Cannot inject token - authToken is NULL!")
        }
    }

    private fun logout() {
        val context = context ?: return
        // Clear all credentials and tokens
        SecureStorageHelper.clearCredentials(context)

        // Navigate to LoginActivity
        val intent = Intent(context, LoginActivity::class.java)
        intent.flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK
        startActivity(intent)
        activity?.finish()
    }

    override fun onDestroyView() {
        super.onDestroyView()
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
            return try {
                context?.let { AuthHelper.getDeviceId(it) } ?: ""
            } catch (e: Exception) { "" }
        }

        @JavascriptInterface
        fun getServerUrl(): String {
            return try {
                context?.let { PreferenceHelper.getServerUrl(it) } ?: ""
            } catch (e: Exception) { "" }
        }

        @JavascriptInterface
        fun refreshToken(): String {
            val context = context ?: return ""
            // This will be called from JavaScript if token expires
            return try {
                AuthHelper.getValidToken(context) ?: ""
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
            val context = context ?: return false
            return NetworkHelper.isNetworkAvailable(context)
        }

        @JavascriptInterface
        fun reloadGallery() {
            activity?.runOnUiThread {
                val context = context
                if (context != null && NetworkHelper.isNetworkAvailable(context)) {
                    loadGallery()
                }
            }
        }

        @JavascriptInterface
        fun startFullBackup(backupType: String) {
            // Allow web app to trigger backups
            activity?.runOnUiThread {
                val intent = Intent(activity, BackupService::class.java).apply {
                    putExtra("backup_type", if (backupType == "video") "video" else "image")
                    putExtra("quick_backup", false)
                }
                activity?.startService(intent)
            }
        }

        @JavascriptInterface
        fun startQuickBackup(backupType: String) {
            // Allow web app to trigger backups
            activity?.runOnUiThread {
                val intent = Intent(activity, BackupService::class.java).apply {
                    putExtra("backup_type", if (backupType == "video") "video" else "image")
                    putExtra("quick_backup", true)
                }
                activity?.startService(intent)
            }
        }
    }
}