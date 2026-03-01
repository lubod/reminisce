package org.openreminisce.app.util

import java.util.concurrent.ConcurrentHashMap

class BackupStatusCache {
    private data class CacheEntry(
        val isBackedUp: Boolean,
        val timestamp: Long
    )

    private val cache = ConcurrentHashMap<String, CacheEntry>()
    private val ttlMillis = 5 * 60 * 1000L // 5 minutes

    companion object {
        @Volatile
        private var instance: BackupStatusCache? = null

        fun getInstance(): BackupStatusCache {
            return instance ?: synchronized(this) {
                instance ?: BackupStatusCache().also { instance = it }
            }
        }
    }

    fun get(fileHash: String): Boolean? {
        val entry = cache[fileHash] ?: return null

        // Check if entry is expired
        if (System.currentTimeMillis() - entry.timestamp > ttlMillis) {
            cache.remove(fileHash)
            return null
        }

        return entry.isBackedUp
    }

    fun put(fileHash: String, isBackedUp: Boolean) {
        cache[fileHash] = CacheEntry(isBackedUp, System.currentTimeMillis())
    }

    fun clear() {
        cache.clear()
    }

    fun size(): Int {
        return cache.size
    }
}
