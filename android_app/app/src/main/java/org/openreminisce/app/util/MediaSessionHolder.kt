package org.openreminisce.app.util

import org.openreminisce.app.model.ImageInfo

/**
 * In-memory holder for large media lists that cannot safely be passed through
 * Android's Intent Binder (~1 MB transaction limit).
 *
 * Populate before calling startActivity(), read in the target Activity's onCreate().
 */
object MediaSessionHolder {
    var hashes: List<String> = emptyList()
    var imageInfos: List<ImageInfo> = emptyList()
    /** Starred state changes from RemoteMediaDetailActivity, keyed by hash. */
    var starredUpdates: MutableMap<String, Boolean> = mutableMapOf()
    /** Hashes deleted in RemoteMediaDetailActivity, to be removed from the grid on return. */
    var deletedHashes: MutableSet<String> = mutableSetOf()

    fun clear() {
        hashes = emptyList()
        imageInfos = emptyList()
    }
}