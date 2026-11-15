package org.openreminisce.app.util

import org.openreminisce.app.model.ImageInfo

/**
 * Holds temporary list/position state when launching a detail viewer so large lists do not exceed
 * Android Binder transaction limits. Delta mutations (stars/deletes) are consumed by the caller
 * fragment on resume even if the detail viewer clears its browsing list on finish.
 */
object MediaSessionHolder {
    var hashes: List<String> = emptyList()
    var imageInfos: List<ImageInfo> = emptyList()

    /** Hashes whose starred state was changed in RemoteMediaDetailActivity: hash -> new isStarred */
    var starredUpdates: MutableMap<String, Boolean> = mutableMapOf()

    /** Hashes deleted in RemoteMediaDetailActivity */
    var deletedHashes: MutableSet<String> = mutableSetOf()

    /** Hashes whose orientation was rotated in RemoteMediaDetailActivity: hash -> new orientation (1..8) */
    var rotatedOrientations: MutableMap<String, Int> = mutableMapOf()

    /** Hashes whose place was updated/cleared in MediaInfoBottomSheetFragment: hash -> new place (or null) */
    var placeUpdates: MutableMap<String, String?> = mutableMapOf()

    /** Set to true when a new derived copy (e.g. AI-enhanced image) is saved to the server library */
    var reloadRequested: Boolean = false

    fun clearMediaSession() {
        hashes = emptyList()
        imageInfos = emptyList()
    }

    fun clear() {
        clearMediaSession()
        starredUpdates.clear()
        deletedHashes.clear()
        rotatedOrientations.clear()
        placeUpdates.clear()
        reloadRequested = false
    }
}
