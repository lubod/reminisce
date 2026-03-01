package org.openreminisce.app

import android.content.Context
import android.content.Intent
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ImageView
import android.widget.TextView
import androidx.recyclerview.widget.RecyclerView
import com.bumptech.glide.Glide
import com.bumptech.glide.load.engine.DiskCacheStrategy
import org.openreminisce.app.model.ThumbnailInfo
import org.openreminisce.app.util.PreferenceHelper
import java.text.SimpleDateFormat
import java.util.*

sealed class RemoteMediaItem {
    data class DateHeader(val date: String, val place: String?) : RemoteMediaItem()
    data class Media(val thumbnailInfo: ThumbnailInfo, val position: Int) : RemoteMediaItem()
}

class RemoteMediaAdapter(
    private val context: Context,
    private val allThumbnails: List<ThumbnailInfo>
) : RecyclerView.Adapter<RecyclerView.ViewHolder>() {

    companion object {
        private const val VIEW_TYPE_HEADER = 0
        private const val VIEW_TYPE_MEDIA = 1
    }

    private val items = mutableListOf<RemoteMediaItem>()
    private val dateFormat = SimpleDateFormat("MMM dd, yyyy", Locale.getDefault())

    init {
        groupByDate()
    }

    private fun groupByDate() {
        items.clear()

        val grouped = allThumbnails.groupBy { thumbnailInfo ->
            try {
                // Parse ISO 8601 date format: "2024-01-15T10:30:00Z"
                val date = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss", Locale.US)
                    .parse(thumbnailInfo.created_at.substring(0, 19))
                dateFormat.format(date ?: Date())
            } catch (e: Exception) {
                "Unknown Date"
            }
        }

        grouped.forEach { (date, thumbnails) ->
            val place = thumbnails.firstOrNull()?.place
            items.add(RemoteMediaItem.DateHeader(date, place))

            thumbnails.forEachIndexed { index, thumbnail ->
                val globalPosition = allThumbnails.indexOf(thumbnail)
                items.add(RemoteMediaItem.Media(thumbnail, globalPosition))
            }
        }
    }

    override fun getItemViewType(position: Int): Int {
        return when (items[position]) {
            is RemoteMediaItem.DateHeader -> VIEW_TYPE_HEADER
            is RemoteMediaItem.Media -> VIEW_TYPE_MEDIA
        }
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): RecyclerView.ViewHolder {
        return when (viewType) {
            VIEW_TYPE_HEADER -> {
                val view = LayoutInflater.from(context)
                    .inflate(R.layout.item_date_header, parent, false)
                DateHeaderViewHolder(view)
            }
            else -> {
                val view = LayoutInflater.from(context)
                    .inflate(R.layout.item_local_media, parent, false)
                MediaViewHolder(view)
            }
        }
    }

    override fun onBindViewHolder(holder: RecyclerView.ViewHolder, position: Int) {
        when (val item = items[position]) {
            is RemoteMediaItem.DateHeader -> {
                (holder as DateHeaderViewHolder).bind(item.date, item.place)
            }
            is RemoteMediaItem.Media -> {
                (holder as MediaViewHolder).bind(item.thumbnailInfo, item.position)
            }
        }
    }

    override fun getItemCount(): Int = items.size

    inner class DateHeaderViewHolder(itemView: View) : RecyclerView.ViewHolder(itemView) {
        private val dateText: TextView = itemView.findViewById(R.id.dateHeaderText)
        private val placeText: TextView = itemView.findViewById(R.id.placeHeaderText)

        fun bind(date: String, place: String?) {
            dateText.text = date

            if (!place.isNullOrEmpty()) {
                placeText.text = if (place.length > 40) {
                    place.substring(0, 37) + "..."
                } else {
                    place
                }
                placeText.visibility = View.VISIBLE
            } else {
                placeText.visibility = View.GONE
            }
        }
    }

    inner class MediaViewHolder(itemView: View) : RecyclerView.ViewHolder(itemView) {
        private val thumbnailImage: ImageView = itemView.findViewById(R.id.thumbnailView)
        private val videoIndicator: ImageView = itemView.findViewById(R.id.mediaTypeIcon)
        private val backupBadge: ImageView = itemView.findViewById(R.id.backupStatusBadge)

        fun bind(thumbnailInfo: ThumbnailInfo, position: Int) {
            val baseUrl = PreferenceHelper.getServerUrl(context)
            val thumbnailUrl = "$baseUrl/api/thumbnail/${thumbnailInfo.hash}"

            // Load thumbnail using Glide with auth headers (configured in GlideModule)
            Glide.with(context)
                .load(thumbnailUrl)
                .diskCacheStrategy(DiskCacheStrategy.ALL)
                .placeholder(R.drawable.ic_image_placeholder)
                .error(R.drawable.ic_broken_image)
                .centerCrop()
                .into(thumbnailImage)

            // Show video indicator for videos
            val isVideo = thumbnailInfo.mediaType == "video"
            if (isVideo) {
                videoIndicator.setImageResource(R.drawable.ic_play_circle)
                videoIndicator.visibility = View.VISIBLE
            } else {
                videoIndicator.visibility = View.GONE
            }

            // Set backup badge (all remote media is backed up)
            backupBadge.setImageResource(R.drawable.ic_check_circle)
            backupBadge.visibility = View.VISIBLE

            // Click to open full preview
            itemView.setOnClickListener {
                val intent = Intent(context, ImagePreviewActivity::class.java).apply {
                    putExtra("IMAGE_HASH", thumbnailInfo.hash)
                    putExtra("POSITION", position)
                    putExtra("TOTAL_COUNT", allThumbnails.size)
                    putExtra("IS_REMOTE", true)
                    putExtra("isVideo", isVideo)
                    // Pass all hashes for swipe navigation
                    putStringArrayListExtra(
                        "ALL_HASHES",
                        ArrayList(allThumbnails.map { it.hash })
                    )
                }
                context.startActivity(intent)
            }
        }
    }
}
