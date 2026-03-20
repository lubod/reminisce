package org.openreminisce.app

import android.content.Context
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.TextView
import androidx.recyclerview.widget.RecyclerView
import com.bumptech.glide.Glide
import com.bumptech.glide.load.engine.DiskCacheStrategy
import org.openreminisce.app.model.ThumbnailInfo
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.model.ImageInfo
import org.openreminisce.app.util.MediaSessionHolder
import java.text.SimpleDateFormat
import java.util.*

sealed class RemoteMediaItem {
    data class DateHeader(val date: String, val place: String?) : RemoteMediaItem()
    data class Media(val thumbnailInfo: ThumbnailInfo, val position: Int) : RemoteMediaItem()
}

class RemoteMediaAdapter(
    private val context: Context,
    thumbnails: List<ThumbnailInfo>,
    private val onMediaClick: (hash: String, position: Int, isVideo: Boolean) -> Unit,
    private val onStarClick: (hash: String, mediaType: String) -> Unit = { _, _ -> }
) : RecyclerView.Adapter<RecyclerView.ViewHolder>() {

    companion object {
        private const val VIEW_TYPE_HEADER = 0
        private const val VIEW_TYPE_MEDIA = 1
    }

    private var allThumbnails: List<ThumbnailInfo> = thumbnails
    private val items = mutableListOf<RemoteMediaItem>()
    private val dateFormat = SimpleDateFormat("MMM dd, yyyy", Locale.getDefault())
    private var groupByPlace: Boolean = false

    init {
        rebuildItems()
    }

    fun updateData(newThumbnails: List<ThumbnailInfo>) {
        allThumbnails = newThumbnails
        rebuildItems()
        notifyDataSetChanged()
    }

    fun setGroupBy(byPlace: Boolean) {
        if (groupByPlace != byPlace) {
            groupByPlace = byPlace
            rebuildItems()
            notifyDataSetChanged()
        }
    }

    private val utcParser = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss", Locale.US).apply {
        timeZone = TimeZone.getTimeZone("UTC")
    }

    fun parseDate(isoDate: String): Date = try {
        utcParser.parse(isoDate.substring(0, minOf(19, isoDate.length))) ?: Date()
    } catch (e: Exception) {
        Date()
    }

    private fun relativeOrFormattedDate(isoDate: String): String {
        val todayCal = Calendar.getInstance()
        val yesterdayCal = Calendar.getInstance().apply { add(Calendar.DAY_OF_YEAR, -1) }

        val today = dateFormat.format(todayCal.time)
        val yesterday = dateFormat.format(yesterdayCal.time)

        return try {
            val key = dateFormat.format(parseDate(isoDate))
            when (key) {
                today -> "Today"
                yesterday -> "Yesterday"
                else -> key
            }
        } catch (e: Exception) {
            "Unknown Date"
        }
    }

    private fun rebuildItems() {
        items.clear()
        if (groupByPlace) {
            groupByPlaceField()
        } else {
            groupByDate()
        }
    }

    private fun groupByDate() {
        val grouped = LinkedHashMap<String, MutableList<Pair<ThumbnailInfo, Int>>>()
        allThumbnails.forEachIndexed { globalIndex, thumb ->
            val key = relativeOrFormattedDate(thumb.created_at)
            grouped.getOrPut(key) { mutableListOf() }.add(thumb to globalIndex)
        }

        grouped.forEach { (date, pairs) ->
            items.add(RemoteMediaItem.DateHeader(date, pairs.first().first.place))
            pairs.forEach { (thumb, globalIndex) ->
                items.add(RemoteMediaItem.Media(thumb, globalIndex))
            }
        }
    }

    private fun groupByPlaceField() {
        val grouped = LinkedHashMap<String, MutableList<Pair<ThumbnailInfo, Int>>>()
        allThumbnails.forEachIndexed { globalIndex, thumb ->
            val key = if (thumb.place.isNullOrEmpty()) "Unknown Place" else thumb.place
            grouped.getOrPut(key) { mutableListOf() }.add(thumb to globalIndex)
        }

        grouped.forEach { (place, pairs) ->
            items.add(RemoteMediaItem.DateHeader(place, null))
            pairs.forEach { (thumb, globalIndex) ->
                items.add(RemoteMediaItem.Media(thumb, globalIndex))
            }
        }
    }

    override fun getItemViewType(position: Int): Int = when (items[position]) {
        is RemoteMediaItem.DateHeader -> VIEW_TYPE_HEADER
        is RemoteMediaItem.Media -> VIEW_TYPE_MEDIA
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): RecyclerView.ViewHolder {
        return when (viewType) {
            VIEW_TYPE_HEADER -> {
                val view = LayoutInflater.from(context).inflate(R.layout.item_date_header, parent, false)
                DateHeaderViewHolder(view)
            }
            else -> {
                val view = LayoutInflater.from(context).inflate(R.layout.item_remote_media, parent, false)
                MediaViewHolder(view)
            }
        }
    }

    override fun onBindViewHolder(holder: RecyclerView.ViewHolder, position: Int) {
        when (val item = items[position]) {
            is RemoteMediaItem.DateHeader -> (holder as DateHeaderViewHolder).bind(item.date, item.place)
            is RemoteMediaItem.Media -> (holder as MediaViewHolder).bind(item.thumbnailInfo, item.position)
        }
    }

    override fun getItemCount(): Int = items.size

    inner class DateHeaderViewHolder(itemView: View) : RecyclerView.ViewHolder(itemView) {
        private val dateText: TextView = itemView.findViewById(R.id.dateHeaderText)
        private val placeText: TextView = itemView.findViewById(R.id.placeHeaderText)

        fun bind(date: String, place: String?) {
            dateText.text = date
            if (!place.isNullOrEmpty()) {
                placeText.text = if (place.length > 40) place.substring(0, 37) + "..." else place
                placeText.visibility = View.VISIBLE
            } else {
                placeText.visibility = View.GONE
            }
        }
    }

    inner class MediaViewHolder(itemView: View) : RecyclerView.ViewHolder(itemView) {
        private val thumbnailImage: ImageView = itemView.findViewById(R.id.thumbnailView)
        private val videoIndicator: ImageView = itemView.findViewById(R.id.mediaTypeIcon)
        private val starButton: ImageButton = itemView.findViewById(R.id.starIcon)
        private val similarityBadge: TextView = itemView.findViewById(R.id.similarityBadge)

        fun bind(thumbnailInfo: ThumbnailInfo, position: Int) {
            val baseUrl = PreferenceHelper.getServerUrl(context)
            val thumbnailUrl = "$baseUrl/api/thumbnail/${thumbnailInfo.hash}"

            Glide.with(context)
                .load(thumbnailUrl)
                .diskCacheStrategy(DiskCacheStrategy.ALL)
                .placeholder(R.drawable.ic_image_placeholder)
                .error(R.drawable.ic_broken_image)
                .centerCrop()
                .into(thumbnailImage)

            // Video indicator
            val isVideo = thumbnailInfo.mediaType == "video"
            videoIndicator.visibility = if (isVideo) View.VISIBLE else View.GONE

            // Star button – always visible, icon reflects state
            starButton.setImageResource(
                if (thumbnailInfo.starred) R.drawable.ic_star else R.drawable.ic_star_outline
            )
            starButton.setOnClickListener {
                onStarClick(thumbnailInfo.hash, thumbnailInfo.mediaType)
            }

            // Similarity badge
            val sim = thumbnailInfo.similarity
            if (sim != null) {
                similarityBadge.text = "${(sim * 100).toInt()}%"
                similarityBadge.visibility = View.VISIBLE
            } else {
                similarityBadge.visibility = View.GONE
            }

            itemView.setOnClickListener {
                // Populate MediaSessionHolder for the detail activity
                MediaSessionHolder.hashes = allThumbnails.map { it.hash }
                MediaSessionHolder.imageInfos = allThumbnails.map { t ->
                    ImageInfo(
                        id = t.hash,
                        date = parseDate(t.created_at),
                        place = t.place,
                        mediaType = t.mediaType
                    )
                }
                onMediaClick(thumbnailInfo.hash, position, isVideo)
            }
        }
    }
}
