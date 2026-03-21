package org.openreminisce.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.method.ScrollingMovementMethod
import android.view.View
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.LogCollector
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.util.SecureStorageHelper
import com.google.android.material.button.MaterialButton
import com.google.android.material.textfield.MaterialAutoCompleteTextView
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class LoginActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "LoginActivity"
        private const val PHASE_SERVER = 0
        private const val PHASE_SETUP = 1
        private const val PHASE_LOGIN = 2
    }

    // Phase containers
    private lateinit var serverPhaseLayout: LinearLayout
    private lateinit var setupPhaseLayout: LinearLayout
    private lateinit var authPhaseLayout: LinearLayout
    private lateinit var continueButton: MaterialButton
    private lateinit var backToServerButton: MaterialButton
    private lateinit var backToServerFromSetupButton: MaterialButton

    // Server phase
    private lateinit var serverUrlLayout: TextInputLayout
    private lateinit var serverUrlInput: MaterialAutoCompleteTextView
    private lateinit var scanQrButton: Button

    // Setup phase
    private lateinit var setupUsernameLayout: TextInputLayout
    private lateinit var setupUsernameInput: TextInputEditText
    private lateinit var setupPasswordLayout: TextInputLayout
    private lateinit var setupPasswordInput: TextInputEditText
    private lateinit var setupConfirmPasswordLayout: TextInputLayout
    private lateinit var setupConfirmPasswordInput: TextInputEditText
    private lateinit var setupActionButton: MaterialButton

    // Login phase
    private lateinit var usernameLayout: TextInputLayout
    private lateinit var usernameInput: TextInputEditText
    private lateinit var passwordLayout: TextInputLayout
    private lateinit var passwordInput: TextInputEditText
    private lateinit var actionButton: Button

    // Shared
    private lateinit var progressBar: ProgressBar
    private lateinit var errorText: TextView

    // Logs UI
    private lateinit var toggleLogsButton: MaterialButton
    private lateinit var logsContainer: LinearLayout
    private lateinit var logsTextView: TextView
    private lateinit var copyLogsButton: MaterialButton
    private lateinit var clearLogsButton: MaterialButton
    private lateinit var refreshLogsButton: MaterialButton
    private lateinit var testConnectionButton: MaterialButton
    private var logsVisible = false

    private var currentPhase = PHASE_SERVER

    private val logListener: (String) -> Unit = { logLine ->
        runOnUiThread {
            logsTextView.append("\n$logLine")
            val scrollAmount = logsTextView.layout?.getLineTop(logsTextView.lineCount) ?: 0
            val scrollY = scrollAmount - logsTextView.height
            if (scrollY > 0) logsTextView.scrollTo(0, scrollY)
        }
    }

    // QR Scanner launcher — on success, fill URL and advance to auth phase
    private val qrScannerLauncher = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { result ->
        if (result.resultCode == RESULT_OK) {
            result.data?.let { data ->
                val serverUrl = data.getStringExtra("server_url")
                if (serverUrl != null) {
                    PreferenceHelper.setServerUrl(this, serverUrl)
                    serverUrlInput.setText(serverUrl)
                    refreshServerUrlAdapter()
                    Toast.makeText(this, "Server URL loaded", Toast.LENGTH_SHORT).show()
                    advanceFromServer()
                }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_login)
        initializeViews()
        setupListeners()
    }

    override fun onResume() {
        super.onResume()
        LogCollector.addListener(logListener)
    }

    override fun onPause() {
        super.onPause()
        LogCollector.removeListener(logListener)
    }

    private fun initializeViews() {
        serverPhaseLayout = findViewById(R.id.serverPhaseLayout)
        setupPhaseLayout = findViewById(R.id.setupPhaseLayout)
        authPhaseLayout = findViewById(R.id.authPhaseLayout)
        continueButton = findViewById(R.id.continueButton)
        backToServerButton = findViewById(R.id.backToServerButton)
        backToServerFromSetupButton = findViewById(R.id.backToServerFromSetupButton)

        serverUrlLayout = findViewById(R.id.serverUrlLayout)
        serverUrlInput = findViewById(R.id.serverUrlInput)
        scanQrButton = findViewById(R.id.scanQrButton)

        setupUsernameLayout = findViewById(R.id.setupUsernameLayout)
        setupUsernameInput = findViewById(R.id.setupUsernameInput)
        setupPasswordLayout = findViewById(R.id.setupPasswordLayout)
        setupPasswordInput = findViewById(R.id.setupPasswordInput)
        setupConfirmPasswordLayout = findViewById(R.id.setupConfirmPasswordLayout)
        setupConfirmPasswordInput = findViewById(R.id.setupConfirmPasswordInput)
        setupActionButton = findViewById(R.id.setupActionButton)

        usernameLayout = findViewById(R.id.usernameLayout)
        usernameInput = findViewById(R.id.usernameInput)
        passwordLayout = findViewById(R.id.passwordLayout)
        passwordInput = findViewById(R.id.passwordInput)
        actionButton = findViewById(R.id.actionButton)

        progressBar = findViewById(R.id.progressBar)
        errorText = findViewById(R.id.errorText)

        toggleLogsButton = findViewById(R.id.toggleLogsButton)
        logsContainer = findViewById(R.id.logsContainer)
        logsTextView = findViewById(R.id.logsTextView)
        copyLogsButton = findViewById(R.id.copyLogsButton)
        clearLogsButton = findViewById(R.id.clearLogsButton)
        refreshLogsButton = findViewById(R.id.refreshLogsButton)
        testConnectionButton = findViewById(R.id.testConnectionButton)

        logsTextView.movementMethod = ScrollingMovementMethod()

        serverUrlInput.setText(PreferenceHelper.getServerUrl(this))
        refreshServerUrlAdapter()
        logsTextView.text = LogCollector.getLogs().ifEmpty { "Logs will appear here..." }
    }

    private fun refreshServerUrlAdapter() {
        val known = PreferenceHelper.getKnownServerUrls(this)
        val adapter = ArrayAdapter(this, android.R.layout.simple_dropdown_item_1line, known)
        serverUrlInput.setAdapter(adapter)
        serverUrlInput.threshold = 0
    }

    private fun setupListeners() {
        continueButton.setOnClickListener {
            val serverUrl = serverUrlInput.text.toString().trim()
            if (serverUrl.isEmpty()) {
                serverUrlLayout.error = "Server URL required"
                return@setOnClickListener
            }
            serverUrlLayout.error = null
            PreferenceHelper.setServerUrl(this, serverUrl)
            advanceFromServer()
        }

        backToServerButton.setOnClickListener { showServerPhase() }
        backToServerFromSetupButton.setOnClickListener { showServerPhase() }

        setupActionButton.setOnClickListener {
            hideError()
            performSetup()
        }

        actionButton.setOnClickListener {
            hideError()
            performLogin()
        }

        scanQrButton.setOnClickListener {
            qrScannerLauncher.launch(Intent(this, QRScannerActivity::class.java))
        }

        toggleLogsButton.setOnClickListener { toggleLogs() }
        copyLogsButton.setOnClickListener { copyLogsToClipboard() }
        clearLogsButton.setOnClickListener {
            LogCollector.clear()
            logsTextView.text = "Logs cleared."
        }
        refreshLogsButton.setOnClickListener { logsTextView.text = LogCollector.getLogs() }
        testConnectionButton.setOnClickListener { testConnection() }
    }

    /** Check setup status and show either setup phase or login phase. */
    private fun advanceFromServer() {
        val serverUrl = PreferenceHelper.getServerUrl(this)
        showLoading()
        CoroutineScope(Dispatchers.IO).launch {
            val needsSetup = try {
                AuthHelper.checkSetupStatus(serverUrl)
            } catch (e: Exception) {
                false
            }
            withContext(Dispatchers.Main) {
                hideLoading()
                if (needsSetup) showSetupPhase() else showLoginPhase()
            }
        }
    }

    private fun showServerPhase() {
        currentPhase = PHASE_SERVER
        serverPhaseLayout.visibility = View.VISIBLE
        setupPhaseLayout.visibility = View.GONE
        authPhaseLayout.visibility = View.GONE
        hideError()
    }

    private fun showSetupPhase() {
        currentPhase = PHASE_SETUP
        serverPhaseLayout.visibility = View.GONE
        setupPhaseLayout.visibility = View.VISIBLE
        authPhaseLayout.visibility = View.GONE
        hideError()
    }

    private fun showLoginPhase() {
        currentPhase = PHASE_LOGIN
        serverPhaseLayout.visibility = View.GONE
        setupPhaseLayout.visibility = View.GONE
        authPhaseLayout.visibility = View.VISIBLE
        hideError()
    }

    private fun toggleLogs() {
        logsVisible = !logsVisible
        if (logsVisible) {
            logsContainer.visibility = View.VISIBLE
            toggleLogsButton.text = "Hide Logs"
            logsTextView.text = LogCollector.getLogs().ifEmpty { "Logs will appear here..." }
            logsTextView.post {
                val scrollAmount = logsTextView.layout?.getLineTop(logsTextView.lineCount) ?: 0
                val scrollY = scrollAmount - logsTextView.height
                if (scrollY > 0) logsTextView.scrollTo(0, scrollY)
            }
        } else {
            logsContainer.visibility = View.GONE
            toggleLogsButton.text = "Show Logs"
        }
    }

    private fun testConnection() {
        val serverUrl = serverUrlInput.text.toString().trim()
        if (serverUrl.isEmpty()) {
            Toast.makeText(this, "Enter Server URL first", Toast.LENGTH_SHORT).show()
            return
        }
        Toast.makeText(this, "Testing connection...", Toast.LENGTH_SHORT).show()
        LogCollector.i(TAG, "Testing connection to $serverUrl")
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val ok = AuthHelper.pingServer(serverUrl)
                LogCollector.i(TAG, if (ok) "Server reachable" else "Server unreachable")
            } catch (e: Exception) {
                LogCollector.e(TAG, "Connection test failed: ${e.message}")
            }
            withContext(Dispatchers.Main) {
                if (logsVisible) logsTextView.text = LogCollector.getLogs()
                Toast.makeText(this@LoginActivity, "Test complete (check logs)", Toast.LENGTH_SHORT).show()
            }
        }
    }

    private fun copyLogsToClipboard() {
        val logs = LogCollector.getLogs()
        if (logs.isEmpty()) {
            Toast.makeText(this, "No logs to copy", Toast.LENGTH_SHORT).show()
            return
        }
        val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("Logs", logs))
        Toast.makeText(this, "Logs copied to clipboard", Toast.LENGTH_SHORT).show()
    }

    private fun performSetup() {
        val username = setupUsernameInput.text.toString().trim()
        val password = setupPasswordInput.text.toString()
        val confirm = setupConfirmPasswordInput.text.toString()

        setupUsernameLayout.error = null
        setupPasswordLayout.error = null
        setupConfirmPasswordLayout.error = null

        if (username.length < 3) { setupUsernameLayout.error = "Min 3 characters"; return }
        if (password.length < 8) { setupPasswordLayout.error = "Min 8 characters"; return }
        if (password != confirm) { setupConfirmPasswordLayout.error = "Passwords do not match"; return }

        val serverUrl = PreferenceHelper.getServerUrl(this)
        showLoading()

        CoroutineScope(Dispatchers.IO).launch {
            val error = AuthHelper.setupAdmin(username, password, serverUrl)
            if (error == null) {
                // Auto-login after setup
                val loginOk = AuthHelper.loginWithCredentials(this@LoginActivity, username, password, serverUrl)
                withContext(Dispatchers.Main) {
                    hideLoading()
                    if (loginOk) {
                        SecureStorageHelper.setUsername(this@LoginActivity, username)
                        SecureStorageHelper.setPassword(this@LoginActivity, password)
                        navigateToMain()
                    } else {
                        showError("Account created. Please sign in.")
                        showLoginPhase()
                    }
                }
            } else {
                withContext(Dispatchers.Main) {
                    hideLoading()
                    showError(error)
                }
            }
        }
    }

    private fun performLogin() {
        val username = usernameInput.text.toString().trim()
        val password = passwordInput.text.toString()

        usernameLayout.error = null
        passwordLayout.error = null

        if (username.isEmpty()) { usernameLayout.error = getString(R.string.username_required); return }
        if (password.isEmpty()) { passwordLayout.error = getString(R.string.password_required); return }

        val serverUrl = PreferenceHelper.getServerUrl(this)
        showLoading()

        CoroutineScope(Dispatchers.IO).launch {
            try {
                val result = AuthHelper.loginWithCredentials(this@LoginActivity, username, password, serverUrl)
                if (result) {
                    SecureStorageHelper.setUsername(this@LoginActivity, username)
                    SecureStorageHelper.setPassword(this@LoginActivity, password)
                    withContext(Dispatchers.Main) {
                        hideLoading()
                        navigateToMain()
                    }
                } else {
                    withContext(Dispatchers.Main) {
                        hideLoading()
                        showError(getString(R.string.login_failed, "Invalid credentials"))
                    }
                }
            } catch (e: Exception) {
                LogCollector.e(TAG, "Error during login", e)
                withContext(Dispatchers.Main) {
                    hideLoading()
                    showError(getString(R.string.login_failed, e.message ?: "Unknown error"))
                }
            }
        }
    }

    private fun showLoading() {
        progressBar.visibility = View.VISIBLE
        actionButton.isEnabled = false
        setupActionButton.isEnabled = false
        continueButton.isEnabled = false
        usernameInput.isEnabled = false
        passwordInput.isEnabled = false
        setupUsernameInput.isEnabled = false
        setupPasswordInput.isEnabled = false
        setupConfirmPasswordInput.isEnabled = false
    }

    private fun hideLoading() {
        progressBar.visibility = View.GONE
        actionButton.isEnabled = true
        setupActionButton.isEnabled = true
        continueButton.isEnabled = true
        usernameInput.isEnabled = true
        passwordInput.isEnabled = true
        setupUsernameInput.isEnabled = true
        setupPasswordInput.isEnabled = true
        setupConfirmPasswordInput.isEnabled = true
    }

    private fun showError(message: String) {
        errorText.text = message
        errorText.visibility = View.VISIBLE
    }

    private fun hideError() {
        errorText.visibility = View.GONE
    }

    private fun navigateToMain() {
        val intent = Intent(this, MainActivity::class.java)
        intent.flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK
        startActivity(intent)
        finish()
    }
}
