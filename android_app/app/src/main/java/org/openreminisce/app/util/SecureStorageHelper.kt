package org.openreminisce.app.util

import android.content.Context
import android.util.Base64
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.security.MessageDigest

class SecureStorageHelper {
    companion object {
        private const val PREFS_NAME = "secure_prefs"
        private const val API_SECRET_KEY = "api_secret"
        private const val ACCESS_TOKEN_KEY = "access_token"
        private const val LOCAL_SERVER_URL_KEY = "local_server_url"
        private const val DARK_THEME_KEY = "dark_theme"
        private const val USERNAME_KEY = "username"
        private const val PASSWORD_KEY = "password"
        private const val EMAIL_KEY = "email"

        @Volatile
        private var cachedPrefs: EncryptedSharedPreferences? = null
        private val lock = Any()

        private fun getEncryptedSharedPreferences(context: Context): EncryptedSharedPreferences {
            // Use cached instance if available (thread-safe double-check locking)
            cachedPrefs?.let { return it }

            synchronized(lock) {
                cachedPrefs?.let { return it }

                val masterKey = MasterKey.Builder(context.applicationContext, MasterKey.DEFAULT_MASTER_KEY_ALIAS)
                    .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                    .build()

                val prefs = EncryptedSharedPreferences.create(
                    context.applicationContext,
                    PREFS_NAME,
                    masterKey,
                    EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                    EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
                ) as EncryptedSharedPreferences

                cachedPrefs = prefs
                return prefs
            }
        }

        fun setApiSecret(context: Context, secret: String) {
            val prefs = getEncryptedSharedPreferences(context)
            prefs.edit().putString(API_SECRET_KEY, secret).apply()
        }

        fun getApiSecret(context: Context): String? {
            val prefs = getEncryptedSharedPreferences(context)
            return prefs.getString(API_SECRET_KEY, null)
        }

        fun clearApiSecret(context: Context) {
            val prefs = getEncryptedSharedPreferences(context)
            prefs.edit().remove(API_SECRET_KEY).remove(ACCESS_TOKEN_KEY).apply()
        }

        fun setAccessToken(context: Context, token: String) {
            val prefs = getEncryptedSharedPreferences(context)
            prefs.edit().putString(ACCESS_TOKEN_KEY, token).apply()
        }

        fun getAccessToken(context: Context): String? {
            val prefs = getEncryptedSharedPreferences(context)
            return prefs.getString(ACCESS_TOKEN_KEY, null)
        }

        fun setLocalServerUrl(context: Context, url: String) {
            val prefs = getEncryptedSharedPreferences(context)
            prefs.edit().putString(LOCAL_SERVER_URL_KEY, url).apply()
        }

        fun getLocalServerUrl(context: Context): String? {
            val prefs = getEncryptedSharedPreferences(context)
            return prefs.getString(LOCAL_SERVER_URL_KEY, null)
        }

        fun setThemePreference(context: Context, darkTheme: Boolean) {
            val prefs = getEncryptedSharedPreferences(context)
            prefs.edit().putBoolean(DARK_THEME_KEY, darkTheme).apply()
        }

        fun getThemePreference(context: Context): Boolean {
            val prefs = getEncryptedSharedPreferences(context)
            return prefs.getBoolean(DARK_THEME_KEY, true) // Default to dark theme
        }

        fun setUsername(context: Context, username: String) {
            val prefs = getEncryptedSharedPreferences(context)
            prefs.edit().putString(USERNAME_KEY, username).apply()
        }

        fun getUsername(context: Context): String? {
            val prefs = getEncryptedSharedPreferences(context)
            return prefs.getString(USERNAME_KEY, null)
        }

        fun setPassword(context: Context, password: String) {
            val prefs = getEncryptedSharedPreferences(context)
            prefs.edit().putString(PASSWORD_KEY, password).apply()
        }

        fun getPassword(context: Context): String? {
            val prefs = getEncryptedSharedPreferences(context)
            return prefs.getString(PASSWORD_KEY, null)
        }

        fun setEmail(context: Context, email: String) {
            val prefs = getEncryptedSharedPreferences(context)
            prefs.edit().putString(EMAIL_KEY, email).apply()
        }

        fun getEmail(context: Context): String? {
            val prefs = getEncryptedSharedPreferences(context)
            return prefs.getString(EMAIL_KEY, null)
        }

        fun clearCredentials(context: Context) {
            val prefs = getEncryptedSharedPreferences(context)
            prefs.edit()
                .remove(USERNAME_KEY)
                .remove(PASSWORD_KEY)
                .remove(EMAIL_KEY)
                .remove(ACCESS_TOKEN_KEY)
                .remove(API_SECRET_KEY)
                .apply()
        }
    }
}