package org.openreminisce.app

import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ImageView
import android.widget.TextView
import androidx.recyclerview.widget.RecyclerView
import com.bumptech.glide.Glide
import org.openreminisce.app.model.ImageInfo
import org.openreminisce.app.model.MediaItem

class ImageAdapter(
    private val items: MutableList<MediaItem> = mutableListOf(),
    private val onImageClick: (ImageInfo) -> Unit,
    private val authToken: String? = null,
    private val deviceId: String? = null
) : RecyclerView.Adapter<RecyclerView.ViewHolder>() {

    companion object {
        private const val TYPE_HEADER = 0
        private const val TYPE_IMAGE = 1
        private const val MAX_LOCATION_CHARS = 40
    }

    // Track which headers have expanded locations
    private val expandedHeaders = mutableSetOf<String>()

    fun updateItems(newItems: List<MediaItem>) {
        val previousSize = items.size
        items.clear()
        items.addAll(newItems)
        if (previousSize > 0) {
            notifyItemRangeRemoved(0, previousSize)
        }
        if (newItems.isNotEmpty()) {
            notifyItemRangeInserted(0, newItems.size)
        }
    }

    fun addItems(newItems: List<MediaItem>) {
        val startPosition = items.size
        items.addAll(newItems)
        notifyItemRangeInserted(startPosition, newItems.size)
    }

    fun clearItems() {
        val size = items.size
        items.clear()
        notifyItemRangeRemoved(0, size)
    }

    override fun getItemViewType(position: Int): Int {
        return when (items[position]) {
            is MediaItem.DateHeader -> TYPE_HEADER
            is MediaItem.Image -> TYPE_IMAGE
        }
    }

    inner class DateHeaderViewHolder(view: View) : RecyclerView.ViewHolder(view) {
        private val dateText: TextView = view.findViewById(R.id.dateHeaderText)
        private val placeText: TextView = view.findViewById(R.id.placeHeaderText)

        fun bind(dateHeader: MediaItem.DateHeader) {
            dateText.text = dateHeader.date

            // Display place information if available
            if (!dateHeader.place.isNullOrEmpty()) {
                val place = dateHeader.place
                val headerId = "${dateHeader.date}_${place}" // Unique ID for this header

                // Update text based on expanded state
                updateLocationDisplay(place, headerId)

                // Set up click listener to toggle expansion
                placeText.setOnClickListener {
                    if (place.length > MAX_LOCATION_CHARS) {
                        // Toggle expanded state
                        if (expandedHeaders.contains(headerId)) {
                            expandedHeaders.remove(headerId)
                        } else {
                            expandedHeaders.add(headerId)
                        }
                        // Update display
                        updateLocationDisplay(place, headerId)
                    }
                }

                placeText.visibility = View.VISIBLE
            } else {
                placeText.visibility = View.GONE
            }
        }

        private fun updateLocationDisplay(place: String, headerId: String) {
            val isExpanded = expandedHeaders.contains(headerId)

            if (place.length > MAX_LOCATION_CHARS && !isExpanded) {
                // Truncate to 40 chars and add ellipsis
                placeText.text = place.take(MAX_LOCATION_CHARS) + "..."
            } else {
                // Show full text
                placeText.text = place
            }
        }
    }

    inner class ImageViewHolder(view: View) : RecyclerView.ViewHolder(view) {
        private val imageView: ImageView = view.findViewById(R.id.imageView)

        fun bind(imageItem: MediaItem.Image) {
            val imageInfo = imageItem.imageInfo

            // Construct the thumbnail URL from the hash
            val baseUrl = org.openreminisce.app.util.PreferenceHelper.getServerUrl(itemView.context)
            val thumbnailUrl = "$baseUrl/api/thumbnail/${imageInfo.id}"

            // Glide will handle authentication via ThumbnailAuthInterceptor in OkHttpClient
            // No need to create GlideUrl with headers every time
            Glide.with(itemView.context)
                .load(thumbnailUrl)
                .placeholder(R.drawable.ic_launcher_foreground)
                .error(R.drawable.ic_launcher_foreground)
                .centerCrop()
                .diskCacheStrategy(com.bumptech.glide.load.engine.DiskCacheStrategy.ALL) // Cache both original & resized
                .skipMemoryCache(false) // Enable memory cache
                .into(imageView)

            imageView.setOnClickListener {
                onImageClick(imageInfo)
            }
        }
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): RecyclerView.ViewHolder {
        return when (viewType) {
            TYPE_HEADER -> {
                val view = LayoutInflater.from(parent.context)
                    .inflate(R.layout.item_date_header, parent, false)
                DateHeaderViewHolder(view)
            }
            TYPE_IMAGE -> {
                val view = LayoutInflater.from(parent.context)
                    .inflate(R.layout.item_image, parent, false)
                ImageViewHolder(view)
            }
            else -> throw IllegalArgumentException("Unknown view type")
        }
    }

    override fun onBindViewHolder(holder: RecyclerView.ViewHolder, position: Int) {
        when (val item = items[position]) {
            is MediaItem.DateHeader -> (holder as DateHeaderViewHolder).bind(item)
            is MediaItem.Image -> (holder as ImageViewHolder).bind(item)
        }
    }

    override fun getItemCount() = items.size
}