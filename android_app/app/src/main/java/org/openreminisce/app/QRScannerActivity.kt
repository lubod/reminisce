package org.openreminisce.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import org.json.JSONObject
import org.openreminisce.app.util.PreferenceHelper

class QRScannerActivity : AppCompatActivity() {

    private val requestPermissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { isGranted ->
        if (isGranted) {
            launchQRScanner()
        } else {
            Toast.makeText(this, "Camera permission required for QR scanning", Toast.LENGTH_SHORT).show()
            finish()
        }
    }

    private val barcodeLauncher = registerForActivityResult(ScanContract()) { result ->
        if (result.contents != null) {
            handleQRCode(result.contents)
        } else {
            finish()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        when {
            ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.CAMERA
            ) == PackageManager.PERMISSION_GRANTED -> {
                launchQRScanner()
            }
            else -> {
                requestPermissionLauncher.launch(Manifest.permission.CAMERA)
            }
        }
    }

    private fun launchQRScanner() {
        val options = ScanOptions()
        options.setDesiredBarcodeFormats(ScanOptions.QR_CODE)
        options.setPrompt("Scan Reminisce QR code")
        options.setBeepEnabled(false)
        options.setOrientationLocked(true)
        options.setCaptureActivity(CustomScannerActivity::class.java)
        barcodeLauncher.launch(options)
    }

    private fun handleQRCode(qrContent: String) {
        try {
            val json = JSONObject(qrContent)
            val urls = mutableListOf<String>()

            // Parse server_urls array (current format)
            if (json.has("server_urls")) {
                val arr = json.getJSONArray("server_urls")
                for (i in 0 until arr.length()) {
                    urls.add(arr.getString(i))
                }
            }
            // Fallback: legacy server_url string
            if (urls.isEmpty() && json.has("server_url")) {
                urls.add(json.getString("server_url"))
            }

            if (urls.isEmpty()) {
                Toast.makeText(this, "No server URL in QR code", Toast.LENGTH_LONG).show()
                finish()
                return
            }

            // Persist all discovered URLs so the login screen can offer them as options
            PreferenceHelper.addKnownServerUrls(this, urls)

            if (urls.size == 1) {
                returnUrl(urls[0])
            } else {
                showUrlSelectionDialog(urls)
            }

        } catch (e: Exception) {
            Toast.makeText(this, "Invalid QR code: ${e.message}", Toast.LENGTH_LONG).show()
            finish()
        }
    }

    private fun showUrlSelectionDialog(urls: List<String>) {
        val labels = urls.map { url -> urlLabel(url) }.toTypedArray()

        AlertDialog.Builder(this)
            .setTitle("Select Server")
            .setItems(labels) { _, which -> returnUrl(urls[which]) }
            .setOnCancelListener { finish() }
            .show()
    }

    private fun urlLabel(url: String): String {
        val host = try {
            java.net.URL(url).host
        } catch (e: Exception) {
            return url
        }
        val isPrivate = host.startsWith("192.168.") ||
            host.startsWith("10.") ||
            host.matches(Regex("^172\\.(1[6-9]|2[0-9]|3[0-1])\\..*")) ||
            host == "localhost" || host == "127.0.0.1"
        return if (isPrivate) "Local Network  —  $url" else "Remote (via VPS)  —  $url"
    }

    private fun returnUrl(url: String) {
        val resultIntent = Intent()
        resultIntent.putExtra("server_url", url)
        setResult(RESULT_OK, resultIntent)
        finish()
    }
}
