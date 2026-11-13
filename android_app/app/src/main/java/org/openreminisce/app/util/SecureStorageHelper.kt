package org.openreminisce.app.util

import android.content.Context
import android.os.Build
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.io.File
import java.security.KeyStore

class SecureStorageHelper {
    companion object {
        private const val TAG = "SecureStorageHelper"
        private const val PREFS_NAME = "secure_prefs"
        private const val API_SECRET_KEY = "api_secret"
        private const val ACCESS_TOKEN_KEY = "access_token"
        private const val REFRESH_TOKEN_KEY = "refresh_token"
        private const val LOCAL_SERVER_URL_KEY = "local_server_url"
        private const val DARK_THEME_KEY = "dark_theme"
        private const val USERNAME_KEY = "username"
        private const val LEGACY_PASSWORD_KEY = "password"
        private const val EMAIL_KEY = "email"
        private const val CERT_PIN_PREFIX = "cert_pin_"

        @Volatile
        private var cachedPrefs: EncryptedSharedPreferences? = null
        private val lock = Any()

        private fun createEncryptedPrefs(appContext: Context): EncryptedSharedPreferences {
            val masterKey = MasterKey.Builder(appContext, MasterKey.DEFAULT_MASTER_KEY_ALIAS)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()

            return EncryptedSharedPreferences.create(
                appContext,
                PREFS_NAME,
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
            ) as EncryptedSharedPreferences
        }

        private fun resetCorruptedStorage(appContext: Context) {
            cachedPrefs = null
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                    appContext.deleteSharedPreferences(PREFS_NAME)
                }
                val prefsFile = File(appContext.filesDir.parentFile, "shared_prefs/$PREFS_NAME.xml")
                if (prefsFile.exists()) prefsFile.delete()
            } catch (e: Exception) {
                Log.w(TAG, "Failed to delete corrupted encrypted prefs file", e)
            }
            try {
                val keyStore = KeyStore.getInstance("AndroidKeyStore")
                keyStore.load(null)
                if (keyStore.containsAlias(MasterKey.DEFAULT_MASTER_KEY_ALIAS)) {
                    keyStore.deleteEntry(MasterKey.DEFAULT_MASTER_KEY_ALIAS)
                }
            } catch (e: Exception) {
                Log.w(TAG, "Failed to remove corrupted MasterKey alias", e)
            }
        }

        private fun getEncryptedSharedPreferences(context: Context): EncryptedSharedPreferences {
            cachedPrefs?.let { return it }

            synchronized(lock) {
                cachedPrefs?.let { return it }
                val appContext = context.applicationContext
                val prefs = try {
                    createEncryptedPrefs(appContext)
                } catch (e: Exception) {
                    // Recover from Android KeyStore invalidation (OS upgrade, biometric/PIN wipe,
                    // or cloud-restored encrypted prefs on another device) rather than crashing in a loop.
                    Log.w(TAG, "EncryptedSharedPreferences corrupted; resetting secure storage", e)
                    resetCorruptedStorage(appContext)
                    createEncryptedPrefs(appContext)
                }
                cachedPrefs = prefs
                return prefs
            }
        }

        fun setApiSecret(context: Context, secret: String) {
            getEncryptedSharedPreferences(context).edit().putString(API_SECRET_KEY, secret).apply()
        }

        fun getApiSecret(context: Context): String? {
            return getEncryptedSharedPreferences(context).getString(API_SECRET_KEY, null)
        }

        fun setAccessToken(context: Context, token: String) {
            getEncryptedSharedPreferences(context).edit()
                .putString(ACCESS_TOKEN_KEY, token)
                .remove(LEGACY_PASSWORD_KEY)
                .apply()
        }

        fun getAccessToken(context: Context): String? {
            return getEncryptedSharedPreferences(context).getString(ACCESS_TOKEN_KEY, null)
        }

        fun setRefreshToken(context: Context, refreshToken: String) {
            getEncryptedSharedPreferences(context).edit()
                .putString(REFRESH_TOKEN_KEY, refreshToken)
                .remove(LEGACY_PASSWORD_KEY)
                .apply()
        }

        fun getRefreshToken(context: Context): String? {
            return getEncryptedSharedPreferences(context).getString(REFRESH_TOKEN_KEY, null)
        }

        fun setLocalServerUrl(context: Context, url: String) {
            getEncryptedSharedPreferences(context).edit().putString(LOCAL_SERVER_URL_KEY, url).apply()
        }

        fun getLocalServerUrl(context: Context): String? {
            return getEncryptedSharedPreferences(context).getString(LOCAL_SERVER_URL_KEY, null)
        }

        fun setThemePreference(context: Context, darkTheme: Boolean) {
            try {
                getEncryptedSharedPreferences(context).edit().putBoolean(DARK_THEME_KEY, darkTheme).apply()
            } catch (e: Exception) {
                Log.w(TAG, "Failed to persist theme preference", e)
            }
        }

        fun getThemePreference(context: Context): Boolean {
            return try {
                getEncryptedSharedPreferences(context).getBoolean(DARK_THEME_KEY, true)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to read theme preference, defaulting dark", e)
                true
            }
        }

        fun setUsername(context: Context, username: String) {
            getEncryptedSharedPreferences(context).edit().putString(USERNAME_KEY, username).apply()
        }

        fun getUsername(context: Context): String? {
            return getEncryptedSharedPreferences(context).getString(USERNAME_KEY, null)
        }

        fun setEmail(context: Context, email: String) {
            getEncryptedSharedPreferences(context).edit().putString(EMAIL_KEY, email).apply()
        }

        fun getEmail(context: Context): String? {
            return getEncryptedSharedPreferences(context).getString(EMAIL_KEY, null)
        }

        fun createPinStore(context: Context): CertificatePinStore {
            val appContext = context.applicationContext
            return object : CertificatePinStore {
                override fun getPin(hostPort: String): String? = getCertPin(appContext, hostPort)
                override fun setPin(hostPort: String, sha256Hex: String) = setCertPin(appContext, hostPort, sha256Hex)
                override fun clearPin(hostPort: String) = clearCertPin(appContext, hostPort)
            }
        }

        fun getCertPin(context: Context, hostPort: String): String? {
            return getEncryptedSharedPreferences(context).getString("$CERT_PIN_PREFIX$hostPort", null)
        }

        fun setCertPin(context: Context, hostPort: String, sha256Hex: String) {
            val normalized = TofuPinningSupport.normalizeSha256(sha256Hex) ?: return
            getEncryptedSharedPreferences(context).edit()
                .putString("$CERT_PIN_PREFIX$hostPort", normalized)
                .apply()
        }

        fun clearCertPin(context: Context, hostPort: String) {
            getEncryptedSharedPreferences(context).edit()
                .remove("$CERT_PIN_PREFIX$hostPort")
                .apply()
        }

        fun clearCredentials(context: Context) {
            getEncryptedSharedPreferences(context).edit()
                .remove(USERNAME_KEY)
                .remove(LEGACY_PASSWORD_KEY)
                .remove(EMAIL_KEY)
                .remove(ACCESS_TOKEN_KEY)
                .remove(REFRESH_TOKEN_KEY)
                .remove(API_SECRET_KEY)
                .apply()
        }
    }
}
