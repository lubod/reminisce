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
}