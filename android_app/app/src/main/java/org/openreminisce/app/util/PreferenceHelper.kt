package org.openreminisce.app.util

import android.content.Context

class PreferenceHelper {
    companion object {
        private const val PREFS_NAME = "my_backup_prefs"

        private const val SERVER_URL_KEY = "server_url"
        private const val BACKUP_TYPE_KEY = "backup_type"
        private const val MEDIA_TYPE_KEY = "media_type"

        private fun getSharedPreferences(context: Context) =
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

        fun setServerUrl(context: Context, url: String) {
            getSharedPreferences(context).edit().putString(SERVER_URL_KEY, url).apply()
        }

        fun getServerUrl(context: Context): String {
            return getSharedPreferences(context).getString(SERVER_URL_KEY, "") ?: ""
        }

        fun isConfigured(context: Context): Boolean {
            return getServerUrl(context).isNotEmpty()
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
