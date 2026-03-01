package org.openreminisce.app.util

import android.util.Log
import java.io.BufferedReader
import java.io.InputStreamReader
import java.text.SimpleDateFormat
import java.util.*
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread

object LogCollector {
    private const val MAX_LOGS = 5000
    private val logs = CopyOnWriteArrayList<String>()
    private val listeners = CopyOnWriteArrayList<(String) -> Unit>()
    private val dateFormat = SimpleDateFormat("HH:mm:ss.SSS", Locale.US)
    private var isLogcatStarted = false

    fun addListener(listener: (String) -> Unit) {
        listeners.add(listener)
        // Immediately trigger logcat reading if not already started
        startLogcatReader()
    }

    fun removeListener(listener: (String) -> Unit) {
        listeners.remove(listener)
    }

    fun log(tag: String, level: String, message: String) {
        // If it's a manual log from Kotlin, we format it.
        // Rust logs will be caught by the logcat reader.
        val timestamp = dateFormat.format(Date())
        val logLine = "[$timestamp] $level/$tag: $message"
        addLogLine(logLine)

        // Also log to Android logcat so it's captured by external tools and potentially our own reader
        when (level) {
            "D" -> Log.d(tag, message)
            "I" -> Log.i(tag, message)
            "W" -> Log.w(tag, message)
            "E" -> Log.e(tag, message)
            else -> Log.v(tag, message)
        }
    }

    private fun addLogLine(line: String) {
        synchronized(logs) {
            // Avoid duplicates if logcat reader catches what we just logged
            if (logs.isNotEmpty() && logs.last() == line) return
            
            logs.add(line)
            if (logs.size > MAX_LOGS) {
                logs.removeAt(0)
            }
        }
        listeners.forEach { it(line) }
    }

    fun d(tag: String, message: String) = log(tag, "D", message)
    fun i(tag: String, message: String) = log(tag, "I", message)
    fun w(tag: String, message: String) = log(tag, "W", message)
    fun e(tag: String, message: String) = log(tag, "E", message)
    fun e(tag: String, message: String, throwable: Throwable) {
        log(tag, "E", "$message: ${throwable.message}")
    }

    fun getLogs(): String {
        return logs.joinToString("\n")
    }

    fun clear() {
        logs.clear()
        listeners.forEach { it("[Logs cleared]") }
    }

    /**
     * Starts a background thread that reads from logcat.
     * This allows us to capture Rust logs (NP2P tag) and system errors.
     */
    @Synchronized
    fun startLogcatReader() {
        if (isLogcatStarted) return
        isLogcatStarted = true

        thread(start = true, isDaemon = true, name = "LogcatReader") {
            try {
                // Filter for our tags or NP2P Rust logs
                // We exclude standard noisy system tags
                val process = Runtime.getRuntime().exec("logcat -v time NP2P:V AuthHelper:V LoginActivity:V MainActivity:V BackupWorker:V *:S")
                val reader = BufferedReader(InputStreamReader(process.inputStream))
                
                var line: String?
                while (true) {
                    line = reader.readLine() ?: break
                    // Logcat output usually has its own timestamp, we just pass it through
                    addLogLine(line)
                }
            } catch (e: Exception) {
                Log.e("LogCollector", "Logcat reader failed", e)
            }
        }
    }

    fun fetchRustLogs() {
        // No-op: captured by startLogcatReader
    }
}
