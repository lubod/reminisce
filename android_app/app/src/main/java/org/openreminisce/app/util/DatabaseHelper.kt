package org.openreminisce.app.util

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import android.util.Log

class DatabaseHelper(context: Context) : SQLiteOpenHelper(context, DATABASE_NAME, null, DATABASE_VERSION) {
    companion object {
        private const val DATABASE_NAME = "my_backup.db"
        private const val DATABASE_VERSION = 2
        
        // Table for file hashes
        private const val TABLE_FILE_HASHES = "file_hashes"
        private const val COLUMN_ID = "id"
        private const val COLUMN_FILE_ID = "file_id"
        private const val COLUMN_HASH = "hash"
        private const val COLUMN_MODIFIED_DATE = "modified_date"
        // Table for confirmed backed up files
        private const val TABLE_BACKED_UP_FILES = "backed_up_files"
        private const val COLUMN_UPLOADED_AT = "uploaded_at"

        
        // Table for backup timestamps
        private const val TABLE_BACKUP_INFO = "backup_info"
        private const val COLUMN_LAST_IMAGE_BACKUP = "last_image_backup_timestamp"
        private const val COLUMN_LAST_VIDEO_BACKUP = "last_video_backup_timestamp"
    }

    override fun onCreate(db: SQLiteDatabase?) {
        // Create file hashes table
        val createFileHashesTable = """
            CREATE TABLE $TABLE_FILE_HASHES (
                $COLUMN_ID INTEGER PRIMARY KEY AUTOINCREMENT,
                $COLUMN_FILE_ID TEXT UNIQUE,
                $COLUMN_HASH TEXT,
                $COLUMN_MODIFIED_DATE INTEGER
            )
        """.trimIndent()
        
        // Create backup info table
        val createBackupInfoTable = """
            CREATE TABLE $TABLE_BACKUP_INFO (
                $COLUMN_ID INTEGER PRIMARY KEY,
                $COLUMN_LAST_IMAGE_BACKUP INTEGER,
                $COLUMN_LAST_VIDEO_BACKUP INTEGER
            )
        """.trimIndent()
        
        db?.execSQL(createFileHashesTable)
        db?.execSQL(createBackupInfoTable)
        // Create backed up files table
        val createBackedUpFilesTable = """
            CREATE TABLE IF NOT EXISTS $TABLE_BACKED_UP_FILES (
                $COLUMN_FILE_ID TEXT PRIMARY KEY,
                $COLUMN_HASH TEXT,
                $COLUMN_UPLOADED_AT INTEGER
            )
        """.trimIndent()
        db?.execSQL(createBackedUpFilesTable)

    }

    override fun onUpgrade(db: SQLiteDatabase?, oldVersion: Int, newVersion: Int) {
        Log.d("DatabaseHelper", "Upgrading database from version $oldVersion to $newVersion")
        // Incremental migrations preserve cached file hashes across schema version updates.
        // Fall back to table recreation only if migrating from unhandled/ancient versions.
        var currentVersion = oldVersion
                if (oldVersion < 2) {
            db?.execSQL("""
                CREATE TABLE IF NOT EXISTS backed_up_files (
                    file_id TEXT PRIMARY KEY,
                    hash TEXT,
                    uploaded_at INTEGER
                )
            """.trimIndent())
        }


        if (currentVersion < newVersion) {
            // Unhandled version migration fallback
            db?.execSQL("DROP TABLE IF EXISTS $TABLE_FILE_HASHES")
            db?.execSQL("DROP TABLE IF EXISTS $TABLE_BACKUP_INFO")
            onCreate(db)
        }
    }

    // Method to insert or update a file hash
    fun insertHash(fileId: String, hash: String, modifiedDate: Long) {
        val db = writableDatabase
        val contentValues = android.content.ContentValues().apply {
            put(COLUMN_FILE_ID, fileId)
            put(COLUMN_HASH, hash)
            put(COLUMN_MODIFIED_DATE, modifiedDate)
        }
        
        // Use replace to insert or update
        db.replace(TABLE_FILE_HASHES, null, contentValues)
    }

    // Method to get a file hash
    fun getHash(fileId: String): String? {
        val db = readableDatabase
        val cursor = db.query(
            TABLE_FILE_HASHES,
            arrayOf(COLUMN_HASH),
            "$COLUMN_FILE_ID = ?",
            arrayOf(fileId),
            null,
            null,
            null
        )

        var hash: String? = null
        if (cursor.moveToFirst()) {
            hash = cursor.getString(0)
        }
        cursor.close()

        return hash
    }

    // Method to get a file hash with modification date validation
    fun getHashIfValid(fileId: String, currentModifiedDate: Long): String? {
        val db = readableDatabase
        val cursor = db.query(
            TABLE_FILE_HASHES,
            arrayOf(COLUMN_HASH, COLUMN_MODIFIED_DATE),
            "$COLUMN_FILE_ID = ?",
            arrayOf(fileId),
            null,
            null,
            null
        )

        var hash: String? = null
        if (cursor.moveToFirst()) {
            val cachedModifiedDate = cursor.getLong(1)
            // Only return hash if the file hasn't been modified
            if (cachedModifiedDate == currentModifiedDate) {
                hash = cursor.getString(0)
                Log.d("DatabaseHelper", "Using cached hash for file $fileId")
            } else {
                Log.d("DatabaseHelper", "File $fileId was modified (cached: $cachedModifiedDate, current: $currentModifiedDate), hash cache invalid")
            }
        }
        cursor.close()

        return hash
    }

    // Method to delete a cached hash (e.g., when hash verification fails)
    fun deleteHash(fileId: String) {
        val db = writableDatabase
        val rowsDeleted = db.delete(TABLE_FILE_HASHES, "$COLUMN_FILE_ID = ?", arrayOf(fileId))
        if (rowsDeleted > 0) {
            Log.d("DatabaseHelper", "Deleted cached hash for file $fileId")
        } else {
            Log.d("DatabaseHelper", "No cached hash found to delete for file $fileId")
        }
    }

    /**
     * Clears all cached hashes from the database.
     * Useful when switching hashing algorithms (e.g., SHA256 -> BLAKE3).
     */
    fun clearAllHashes() {
        val db = writableDatabase
        db.delete(TABLE_FILE_HASHES, null, null)
        Log.d("DatabaseHelper", "Cleared all cached hashes from database")
    }

    // Method to save the last image backup timestamp
    fun saveLastImageBackupTimestamp(timestamp: Long) {
        val db = writableDatabase

        // Check if row exists
        val cursor = db.query(
            TABLE_BACKUP_INFO,
            arrayOf(COLUMN_ID),
            "$COLUMN_ID = ?",
            arrayOf("1"),
            null,
            null,
            null
        )

        val rowExists = cursor.moveToFirst()
        cursor.close()

        if (rowExists) {
            // Update existing row, only updating the image timestamp column
            val contentValues = android.content.ContentValues().apply {
                put(COLUMN_LAST_IMAGE_BACKUP, timestamp)
            }
            db.update(TABLE_BACKUP_INFO, contentValues, "$COLUMN_ID = ?", arrayOf("1"))
        } else {
            // Insert new row
            val contentValues = android.content.ContentValues().apply {
                put(COLUMN_ID, 1)
                put(COLUMN_LAST_IMAGE_BACKUP, timestamp)
            }
            db.insert(TABLE_BACKUP_INFO, null, contentValues)
        }
    }

    // Method to save the last video backup timestamp
    fun saveLastVideoBackupTimestamp(timestamp: Long) {
        val db = writableDatabase

        // Check if row exists
        val cursor = db.query(
            TABLE_BACKUP_INFO,
            arrayOf(COLUMN_ID),
            "$COLUMN_ID = ?",
            arrayOf("1"),
            null,
            null,
            null
        )

        val rowExists = cursor.moveToFirst()
        cursor.close()

        if (rowExists) {
            // Update existing row, only updating the video timestamp column
            val contentValues = android.content.ContentValues().apply {
                put(COLUMN_LAST_VIDEO_BACKUP, timestamp)
            }
            db.update(TABLE_BACKUP_INFO, contentValues, "$COLUMN_ID = ?", arrayOf("1"))
        } else {
            // Insert new row
            val contentValues = android.content.ContentValues().apply {
                put(COLUMN_ID, 1)
                put(COLUMN_LAST_VIDEO_BACKUP, timestamp)
            }
            db.insert(TABLE_BACKUP_INFO, null, contentValues)
        }
    }

    // Method to get the last image backup timestamp
    fun getLastImageBackupTimestamp(): Long? {
        val db = readableDatabase
        val cursor = db.query(
            TABLE_BACKUP_INFO,
            arrayOf(COLUMN_LAST_IMAGE_BACKUP),
            "$COLUMN_ID = ?",
            arrayOf("1"),
            null,
            null,
            null
        )
        
        var timestamp: Long? = null
        if (cursor.moveToFirst()) {
            val columnIndex = cursor.getColumnIndex(COLUMN_LAST_IMAGE_BACKUP)
            if (columnIndex >= 0) {
                val value = cursor.getLong(columnIndex)
                if (value > 0) { // Only return valid timestamps
                    timestamp = value
                }
            }
        }
        cursor.close()
        
        return timestamp
    }

    // Method to get the last video backup timestamp
    fun getLastVideoBackupTimestamp(): Long? {
        val db = readableDatabase
        val cursor = db.query(
            TABLE_BACKUP_INFO,
            arrayOf(COLUMN_LAST_VIDEO_BACKUP),
            "$COLUMN_ID = ?",
            arrayOf("1"),
            null,
            null,
            null
        )

        var timestamp: Long? = null
        if (cursor.moveToFirst()) {
            val columnIndex = cursor.getColumnIndex(COLUMN_LAST_VIDEO_BACKUP)
            if (columnIndex >= 0) {
                val value = cursor.getLong(columnIndex)
                if (value > 0) { // Only return valid timestamps
                    timestamp = value
                }
            }
        }
        cursor.close()

        return timestamp
    }

    /**
     * Data class to hold file ID with its cached hash and modified date
     */
    data class CachedHashInfo(
        val fileId: String,
        val hash: String,
        val modifiedDate: Long
    )

    /**
     * Bulk query to get all cached hashes with their modified dates
     * This is much more efficient than calling getHashIfValid for each file
     * @return Map of fileId to CachedHashInfo
     */
    fun getAllCachedHashes(): Map<String, CachedHashInfo> {
        val db = readableDatabase
        val cursor = db.query(
            TABLE_FILE_HASHES,
            arrayOf(COLUMN_FILE_ID, COLUMN_HASH, COLUMN_MODIFIED_DATE),
            null,
            null,
            null,
            null,
            null
        )

        val cachedHashes = mutableMapOf<String, CachedHashInfo>()
        while (cursor.moveToNext()) {
            val fileId = cursor.getString(0)
            val hash = cursor.getString(1)
            val modifiedDate = cursor.getLong(2)
            cachedHashes[fileId] = CachedHashInfo(fileId, hash, modifiedDate)
        }
        cursor.close()

        Log.d("DatabaseHelper", "Retrieved ${cachedHashes.size} cached hashes from database")
        return cachedHashes
    }

    // Mark a file as confirmed backed up on the server
    fun markFileBackedUp(fileId: String, hash: String, uploadedAt: Long = System.currentTimeMillis()) {
        val db = writableDatabase
        val contentValues = android.content.ContentValues().apply {
            put(COLUMN_FILE_ID, fileId)
            put(COLUMN_HASH, hash)
            put(COLUMN_UPLOADED_AT, uploadedAt)
        }
        db.replace(TABLE_BACKED_UP_FILES, null, contentValues)
        Log.d("DatabaseHelper", "Marked file $fileId as backed up")
    }

    // Check if a file is confirmed backed up
    fun isMediaBackedUp(fileId: String): Boolean {
        val db = readableDatabase
        val cursor = db.query(
            TABLE_BACKED_UP_FILES,
            arrayOf(COLUMN_FILE_ID),
            "$COLUMN_FILE_ID = ?",
            arrayOf(fileId),
            null,
            null,
            null
        )
        val exists = cursor.moveToFirst()
        cursor.close()
        return exists
    }

    // Retrieve all backed up file IDs in a single bulk query
    fun getAllBackedUpFileIds(): Set<String> {
        val db = readableDatabase
        val cursor = db.query(
            TABLE_BACKED_UP_FILES,
            arrayOf(COLUMN_FILE_ID),
            null,
            null,
            null,
            null,
            null
        )
        val set = mutableSetOf<String>()
        while (cursor.moveToNext()) {
            set.add(cursor.getString(0))
        }
        cursor.close()
        Log.d("DatabaseHelper", "Retrieved ${set.size} confirmed backed up files from database")
        return set
    }
}
