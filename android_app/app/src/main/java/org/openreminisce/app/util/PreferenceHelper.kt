package org.openreminisce.app.util

import android.content.Context
import org.json.JSONArray

class PreferenceHelper {
    companion object {
        private const val PREFS_NAME = "my_backup_prefs"

        private const val SERVER_URL_KEY = "server_url"
        private const val KNOWN_SERVER_URLS_KEY = "known_server_urls"
        private const val BACKUP_TYPE_KEY = "backup_type"
        private const val MEDIA_TYPE_KEY = "media_type"

        private fun getSharedPreferences(context: Context) =
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

        fun setServerUrl(context: Context, url: String) {
            getSharedPreferences(context).edit().putString(SERVER_URL_KEY, url).apply()
            // also ensure it's in the known list
            addKnownServerUrls(context, listOf(url))
        }

        fun getServerUrl(context: Context): String {
            return getSharedPreferences(context).getString(SERVER_URL_KEY, "") ?: ""
        }

        fun isConfigured(context: Context): Boolean {
            return getServerUrl(context).isNotEmpty()
        }

        /** Persist a list of server URLs, deduplicating. Most-recently-added goes to the front. */
        fun addKnownServerUrls(context: Context, urls: List<String>) {
            val existing = getKnownServerUrls(context).toMutableList()
            // prepend new URLs (in reverse so first stays first), removing dupes
            for (url in urls.reversed()) {
                existing.remove(url)
                existing.add(0, url)
            }
            val arr = JSONArray()
            existing.forEach { arr.put(it) }
            getSharedPreferences(context).edit()
                .putString(KNOWN_SERVER_URLS_KEY, arr.toString())
                .apply()
        }

        fun getKnownServerUrls(context: Context): List<String> {
            val raw = getSharedPreferences(context).getString(KNOWN_SERVER_URLS_KEY, null)
                ?: return emptyList()
            return try {
                val arr = JSONArray(raw)
                (0 until arr.length()).map { arr.getString(it) }
            } catch (e: Exception) {
                emptyList()
            }
        }

        fun setBackupType(context: Context, backupType: String) {
            getSharedPreferences(context).edit().putString(BACKUP_TYPE_KEY, backupType).apply()
        }

        fun getBackupType(context: Context): String {
            return getSharedPreferences(context).getString(BACKUP_TYPE_KEY, "image") ?: "image"
        }

        fun setMediaType(context: Context, mediaType: String) {
            getSharedPreferences(context).edit().putString(MEDIA_TYPE_KEY, mediaType).apply()
        }

        fun getMediaType(context: Context): String {
            return getSharedPreferences(context).getString(MEDIA_TYPE_KEY, "all") ?: "all"
        }
    }
}
