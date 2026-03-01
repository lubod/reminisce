package org.openreminisce.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.method.ScrollingMovementMethod
import android.util.Patterns
import android.view.View
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
import com.google.android.material.tabs.TabLayout
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class LoginActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "LoginActivity"
        private const val MODE_LOGIN = 0
        private const val MODE_REGISTER = 1
    }

    private lateinit var tabLayout: TabLayout
    private lateinit var serverUrlLayout: TextInputLayout
    private lateinit var serverUrlInput: TextInputEditText
    private lateinit var usernameLayout: TextInputLayout
    private lateinit var usernameInput: TextInputEditText
    private lateinit var emailLayout: TextInputLayout
    private lateinit var emailInput: TextInputEditText
    private lateinit var passwordLayout: TextInputLayout
    private lateinit var passwordInput: TextInputEditText
    private lateinit var actionButton: Button
    private lateinit var scanQrButton: Button
    private lateinit var pasteJsonButton: Button
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

    private var currentMode = MODE_LOGIN

    private val logListener: (String) -> Unit = { logLine ->
        runOnUiThread {
            logsTextView.append("\n$logLine")
            val scrollAmount = logsTextView.layout?.getLineTop(logsTextView.lineCount) ?: 0
            val scrollY = scrollAmount - logsTextView.height
            if (scrollY > 0) logsTextView.scrollTo(0, scrollY)
        }
    }

    // QR Scanner launcher
    private val qrScannerLauncher = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { result ->
        if (result.resultCode == RESULT_OK) {
            result.data?.let { data ->
                val serverUrl = data.getStringExtra("server_url")
                if (serverUrl != null) {
                    PreferenceHelper.setServerUrl(this, serverUrl)
                    serverUrlInput.setText(serverUrl)
                    Toast.makeText(this, "Server URL loaded", Toast.LENGTH_SHORT).show()
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

    private fun initializeViews() {
        tabLayout = findViewById(R.id.authTabLayout)
        serverUrlLayout = findViewById(R.id.serverUrlLayout)
        serverUrlInput = findViewById(R.id.serverUrlInput)
        usernameLayout = findViewById(R.id.usernameLayout)
        usernameInput = findViewById(R.id.usernameInput)
        emailLayout = findViewById(R.id.emailLayout)
        emailInput = findViewById(R.id.emailInput)
        passwordLayout = findViewById(R.id.passwordLayout)
        passwordInput = findViewById(R.id.passwordInput)
        actionButton = findViewById(R.id.actionButton)
        scanQrButton = findViewById(R.id.scanQrButton)
        pasteJsonButton = findViewById(R.id.pasteJsonButton)
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
        logsTextView.text = LogCollector.getLogs().ifEmpty { "Logs will appear here..." }
    }

    private fun setupListeners() {
        tabLayout.addOnTabSelectedListener(object : TabLayout.OnTabSelectedListener {
            override fun onTabSelected(tab: TabLayout.Tab?) {
                when (tab?.position) {
                    MODE_LOGIN -> switchToLoginMode()
                    MODE_REGISTER -> switchToRegisterMode()
                }
            }
            override fun onTabUnselected(tab: TabLayout.Tab?) {}
            override fun onTabReselected(tab: TabLayout.Tab?) {}
        })

        actionButton.setOnClickListener {
            hideError()
            when (currentMode) {
                MODE_LOGIN -> performLogin()
                MODE_REGISTER -> performRegistration()
            }
        }

        scanQrButton.setOnClickListener {
            qrScannerLauncher.launch(Intent(this, QRScannerActivity::class.java))
        }

        pasteJsonButton.setOnClickListener { testConnection() }

        toggleLogsButton.setOnClickListener { toggleLogs() }
        copyLogsButton.setOnClickListener { copyLogsToClipboard() }
        clearLogsButton.setOnClickListener {
            LogCollector.clear()
            logsTextView.text = "Logs cleared."
        }
        refreshLogsButton.setOnClickListener { logsTextView.text = LogCollector.getLogs() }
        testConnectionButton.setOnClickListener { testConnection() }
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

    private fun switchToLoginMode() {
        currentMode = MODE_LOGIN
        emailLayout.visibility = View.GONE
        actionButton.text = getString(R.string.login)
        hideError()
    }

    private fun switchToRegisterMode() {
        currentMode = MODE_REGISTER
        emailLayout.visibility = View.VISIBLE
        actionButton.text = getString(R.string.register)
        hideError()
    }

    private fun performLogin() {
        if (!validateInputs()) return

        val serverUrl = serverUrlInput.text.toString().trim()
        val username = usernameInput.text.toString().trim()
        val password = passwordInput.text.toString()

        PreferenceHelper.setServerUrl(this, serverUrl)
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

    private fun performRegistration() {
        if (!validateInputs()) return

        val serverUrl = serverUrlInput.text.toString().trim()
        val username = usernameInput.text.toString().trim()
        val email = emailInput.text.toString().trim()
        val password = passwordInput.text.toString()

        PreferenceHelper.setServerUrl(this, serverUrl)
        showLoading()

        CoroutineScope(Dispatchers.IO).launch {
            try {
                val result = AuthHelper.registerUser(username, email, password, serverUrl)
                withContext(Dispatchers.Main) {
                    hideLoading()
                    if (result) {
                        SecureStorageHelper.setUsername(this@LoginActivity, username)
                        SecureStorageHelper.setPassword(this@LoginActivity, password)
                        SecureStorageHelper.setEmail(this@LoginActivity, email)
                        performLogin()
                    } else {
                        showError(getString(R.string.registration_failed, "Registration failed"))
                    }
                }
            } catch (e: Exception) {
                LogCollector.e(TAG, "Error during registration", e)
                withContext(Dispatchers.Main) {
                    hideLoading()
                    showError(getString(R.string.registration_failed, e.message ?: "Unknown error"))
                }
            }
        }
    }

    private fun validateInputs(): Boolean {
        var isValid = true

        val serverUrl = serverUrlInput.text.toString().trim()
        if (serverUrl.isEmpty()) {
            serverUrlLayout.error = "Server URL required"
            isValid = false
        } else {
            serverUrlLayout.error = null
        }

        val username = usernameInput.text.toString().trim()
        if (username.isEmpty()) {
            usernameLayout.error = getString(R.string.username_required)
            isValid = false
        } else {
            usernameLayout.error = null
        }

        val password = passwordInput.text.toString()
        if (password.isEmpty()) {
            passwordLayout.error = getString(R.string.password_required)
            isValid = false
        } else {
            passwordLayout.error = null
        }

        if (currentMode == MODE_REGISTER) {
            val email = emailInput.text.toString().trim()
            if (email.isEmpty() || !Patterns.EMAIL_ADDRESS.matcher(email).matches()) {
                emailLayout.error = getString(R.string.invalid_email)
                isValid = false
            } else {
                emailLayout.error = null
            }
        }

        return isValid
    }

    private fun showLoading() {
        progressBar.visibility = View.VISIBLE
        actionButton.isEnabled = false
        serverUrlInput.isEnabled = false
        usernameInput.isEnabled = false
        emailInput.isEnabled = false
        passwordInput.isEnabled = false
    }

    private fun hideLoading() {
        progressBar.visibility = View.GONE
        actionButton.isEnabled = true
        serverUrlInput.isEnabled = true
        usernameInput.isEnabled = true
        emailInput.isEnabled = true
        passwordInput.isEnabled = true
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
