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

            when {
                urls.isEmpty() -> {
                    Toast.makeText(this, "No server URL in QR code", Toast.LENGTH_LONG).show()
                    finish()
                }
                urls.size == 1 -> returnUrl(urls[0])
                else -> showUrlSelectionDialog(urls)
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
            url
        }
        // Netbird uses the 100.64.0.0/10 CGNAT range
        val isNetbird = host.matches(Regex("^100\\.(6[4-9]|[7-9]\\d|1[01]\\d|12[0-7])\\..*"))
        return if (isNetbird) "Netbird VPN  —  $url" else "Local Network  —  $url"
    }

    private fun returnUrl(url: String) {
        val resultIntent = Intent()
        resultIntent.putExtra("server_url", url)
        setResult(RESULT_OK, resultIntent)
        finish()
    }
}
