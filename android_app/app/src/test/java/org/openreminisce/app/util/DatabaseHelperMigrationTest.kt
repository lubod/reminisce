package org.openreminisce.app.util

import android.app.Application
import android.database.sqlite.SQLiteDatabase
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33], application = Application::class)
class DatabaseHelperMigrationTest {

    @After
    fun cleanupDatabase() {
        val context = RuntimeEnvironment.getApplication()
        context.deleteDatabase("my_backup.db")
    }

    @Test
    fun upgradeFromVersion1ToVersion2PreservesFileHashesAndTimestamps() {
        val context = RuntimeEnvironment.getApplication()
        val dbFile = context.getDatabasePath("my_backup.db")
        dbFile.parentFile?.mkdirs()

        SQLiteDatabase.openOrCreateDatabase(dbFile.absolutePath, null).use { v1Db ->
            v1Db.execSQL(
                """
                CREATE TABLE file_hashes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    file_id TEXT UNIQUE,
                    hash TEXT,
                    modified_date INTEGER
                )
                """.trimIndent()
            )
            v1Db.execSQL(
                """
                CREATE TABLE backup_info (
                    id INTEGER PRIMARY KEY,
                    last_image_backup_timestamp INTEGER,
                    last_video_backup_timestamp INTEGER
                )
                """.trimIndent()
            )
            v1Db.execSQL("INSERT INTO file_hashes (file_id, hash, modified_date) VALUES ('101', 'abc123', 1700000100)")
            v1Db.execSQL("INSERT INTO backup_info (id, last_image_backup_timestamp, last_video_backup_timestamp) VALUES (1, 1700000200, 1700000300)")
            v1Db.version = 1
        }

        val helper = DatabaseHelper(context)
        try {
            // Version 1 rows and timestamps must survive v1 -> v2 upgrade
            assertEquals("abc123", helper.getHashIfValid("101", 1700000100L))
            assertEquals(1700000200L, helper.getLastImageBackupTimestamp())
            assertEquals(1700000300L, helper.getLastVideoBackupTimestamp())

            // New v2 table must be present and usable
            assertFalse(helper.isMediaBackedUp("101"))
            helper.markFileBackedUp("101", "abc123")
            assertTrue(helper.isMediaBackedUp("101"))
        } finally {
            helper.close()
        }
    }
}
