#!/bin/bash

# Script to generate native Kotlin remote gallery tab
# This replaces the WebView-based RemoteMediaFragment with a native implementation
# Based on LocalMediaFragment structure

set -e

echo "Generating native remote gallery implementation..."

# Define paths
APP_DIR="/Users/ldr/work/reminisce/android_app/app/src/main"
KOTLIN_DIR="$APP_DIR/java/org.openreminisce.app"
RES_DIR="$APP_DIR/res"
LAYOUT_DIR="$RES_DIR/layout"

# 1. Create RemoteMediaRepository.kt
echo "Creating RemoteMediaRepository.kt..."
cat > "$KOTLIN_DIR/repository/RemoteMediaRepository.kt" << 'EOF'
package org.openreminisce.app.repository

import android.content.Context
import android.util.Log
import org.openreminisce.app.model.ThumbnailInfo
import org.openreminisce.app.model.ThumbnailResponse
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.PreferenceHelper
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.HttpURLConnection
import java.net.URL

class RemoteMediaRepository(private val context: Context) {

    companion object {
        private const val TAG = "RemoteMediaRepository"
        private const val PAGE_LIMIT = 100
    }

    suspend fun fetchRemoteThumbnails(
        page: Int = 1,
        limit: Int = PAGE_LIMIT
    ): Result<ThumbnailResponse> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context)
            val deviceId = PreferenceHelper.getDeviceId(context)

            if (serverUrl.isEmpty() || token == null) {
                return@withContext Result.failure(Exception("Server not configured or not authenticated"))
            }

            // API endpoint to fetch thumbnails - adjust based on your backend API
            val apiUrl = "$serverUrl/api/thumbnails?page=$page&limit=$limit"
            Log.d(TAG, "Fetching thumbnails from: $apiUrl")

            val url = URL(apiUrl)
            val connection = url.openConnection() as HttpURLConnection

            connection.apply {
                requestMethod = "GET"
                setRequestProperty("Authorization", "Bearer $token")
                setRequestProperty("Cookie", "X-Device-ID=$deviceId")
                setRequestProperty("Accept", "application/json")
                connectTimeout = 30000
                readTimeout = 30000
            }

            val responseCode = connection.responseCode
            if (responseCode == HttpURLConnection.HTTP_OK) {
                val reader = BufferedReader(InputStreamReader(connection.inputStream))
                val response = reader.readText()
                reader.close()

                val thumbnailResponse = parseThumbnailResponse(response)
                Log.d(TAG, "Fetched ${thumbnailResponse.thumbnails.size} thumbnails (page $page)")
                Result.success(thumbnailResponse)
            } else {
                val errorStream = connection.errorStream
                val error = if (errorStream != null) {
                    BufferedReader(InputStreamReader(errorStream)).readText()
                } else {
                    "HTTP $responseCode"
                }
                Log.e(TAG, "Failed to fetch thumbnails: $error")
                Result.failure(Exception("Failed to fetch thumbnails: $error"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error fetching remote thumbnails", e)
            Result.failure(e)
        }
    }

    private fun parseThumbnailResponse(json: String): ThumbnailResponse {
        val jsonObject = JSONObject(json)
        val thumbnailsArray = jsonObject.getJSONArray("thumbnails")
        val thumbnails = mutableListOf<ThumbnailInfo>()

        for (i in 0 until thumbnailsArray.length()) {
            val item = thumbnailsArray.getJSONObject(i)
            thumbnails.add(
                ThumbnailInfo(
                    hash = item.getString("hash"),
                    created_at = item.getString("created_at"),
                    place = item.optString("place").takeIf { it.isNotEmpty() }
                )
            )
        }

        return ThumbnailResponse(
            thumbnails = thumbnails,
            total = jsonObject.getInt("total"),
            page = jsonObject.getInt("page"),
            limit = jsonObject.getInt("limit")
        )
    }
}
EOF

# 2. Create RemoteMediaAdapter.kt
echo "Creating RemoteMediaAdapter.kt..."
cat > "$KOTLIN_DIR/RemoteMediaAdapter.kt" << 'EOF'
package org.openreminisce.app

import android.content.Context
import android.content.Intent
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ImageView
import android.widget.TextView
import androidx.cardview.widget.CardView
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
                    .inflate(R.layout.item_media, parent, false)
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
        private val placeText: TextView = itemView.findViewById(R.id.placeText)

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
        private val cardView: CardView = itemView.findViewById(R.id.mediaCardView)
        private val thumbnailImage: ImageView = itemView.findViewById(R.id.thumbnailImage)
        private val videoIndicator: ImageView = itemView.findViewById(R.id.videoIndicator)
        private val backupBadge: ImageView = itemView.findViewById(R.id.backupBadge)

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

            // All remote media is backed up by definition
            backupBadge.visibility = View.VISIBLE

            // Hide video indicator for now (would need mediaType field in ThumbnailInfo)
            videoIndicator.visibility = View.GONE

            // Click to open full preview
            cardView.setOnClickListener {
                val intent = Intent(context, ImagePreviewActivity::class.java).apply {
                    putExtra("IMAGE_HASH", thumbnailInfo.hash)
                    putExtra("POSITION", position)
                    putExtra("TOTAL_COUNT", allThumbnails.size)
                    putExtra("IS_REMOTE", true)
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
EOF

# 3. Create new RemoteMediaFragment.kt (native version)
echo "Creating native RemoteMediaFragment.kt..."
cat > "$KOTLIN_DIR/fragments/RemoteMediaFragmentNative.kt" << 'EOF'
package org.openreminisce.app.fragments

import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ProgressBar
import android.widget.TextView
import androidx.fragment.app.Fragment
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.GridLayoutManager
import androidx.recyclerview.widget.RecyclerView
import androidx.swiperefreshlayout.widget.SwipeRefreshLayout
import org.openreminisce.app.R
import org.openreminisce.app.RemoteMediaAdapter
import org.openreminisce.app.model.ThumbnailInfo
import org.openreminisce.app.repository.RemoteMediaRepository
import com.google.android.material.floatingactionbutton.FloatingActionButton
import kotlinx.coroutines.launch

class RemoteMediaFragmentNative : Fragment() {

    private lateinit var recyclerView: RecyclerView
    private lateinit var progressBar: ProgressBar
    private lateinit var swipeRefresh: SwipeRefreshLayout
    private lateinit var emptyStateText: TextView
    private lateinit var refreshFab: FloatingActionButton
    private lateinit var repository: RemoteMediaRepository

    private var allThumbnails = listOf<ThumbnailInfo>()
    private var currentPage = 1
    private var isLoading = false
    private var hasMore = true

    override fun onCreateView(
        inflater: LayoutInflater,
        container: ViewGroup?,
        savedInstanceState: Bundle?
    ): View? {
        return inflater.inflate(R.layout.fragment_remote_media_native, container, false)
    }

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        recyclerView = view.findViewById(R.id.remoteMediaRecyclerView)
        progressBar = view.findViewById(R.id.remoteMediaProgressBar)
        swipeRefresh = view.findViewById(R.id.remoteMediaSwipeRefresh)
        emptyStateText = view.findViewById(R.id.remoteMediaEmptyState)
        refreshFab = view.findViewById(R.id.remoteRefreshFab)

        repository = RemoteMediaRepository(requireContext())

        setupRecyclerView()
        setupSwipeRefresh()
        setupFab()

        loadRemoteMedia()
    }

    private fun setupRecyclerView() {
        val gridLayoutManager = GridLayoutManager(requireContext(), 3).apply {
            spanSizeLookup = object : GridLayoutManager.SpanSizeLookup() {
                override fun getSpanSize(position: Int): Int {
                    // Date headers span all 3 columns
                    val adapter = recyclerView.adapter as? RemoteMediaAdapter
                    return if (adapter != null && position < adapter.itemCount) {
                        if (adapter.getItemViewType(position) == 0) 3 else 1
                    } else {
                        1
                    }
                }
            }
        }

        recyclerView.apply {
            layoutManager = gridLayoutManager
            setHasFixedSize(true)

            // Add scroll listener for pagination
            addOnScrollListener(object : RecyclerView.OnScrollListener() {
                override fun onScrolled(recyclerView: RecyclerView, dx: Int, dy: Int) {
                    super.onScrolled(recyclerView, dx, dy)

                    if (!isLoading && hasMore && dy > 0) {
                        val visibleItemCount = gridLayoutManager.childCount
                        val totalItemCount = gridLayoutManager.itemCount
                        val firstVisibleItemPosition = gridLayoutManager.findFirstVisibleItemPosition()

                        if ((visibleItemCount + firstVisibleItemPosition) >= totalItemCount - 10) {
                            loadMoreMedia()
                        }
                    }
                }
            })
        }
    }

    private fun setupSwipeRefresh() {
        swipeRefresh.setOnRefreshListener {
            refreshMedia()
        }
    }

    private fun setupFab() {
        refreshFab.setOnClickListener {
            refreshMedia()
        }
    }

    private fun loadRemoteMedia() {
        if (isLoading) return

        isLoading = true
        progressBar.visibility = View.VISIBLE
        emptyStateText.visibility = View.GONE

        lifecycleScope.launch {
            repository.fetchRemoteThumbnails(page = currentPage).fold(
                onSuccess = { response ->
                    allThumbnails = allThumbnails + response.thumbnails
                    hasMore = allThumbnails.size < response.total

                    updateUI()
                    isLoading = false
                    progressBar.visibility = View.GONE
                    swipeRefresh.isRefreshing = false
                },
                onFailure = { error ->
                    isLoading = false
                    progressBar.visibility = View.GONE
                    swipeRefresh.isRefreshing = false

                    if (allThumbnails.isEmpty()) {
                        emptyStateText.text = "Failed to load remote media: ${error.message}"
                        emptyStateText.visibility = View.VISIBLE
                    }
                }
            )
        }
    }

    private fun loadMoreMedia() {
        currentPage++
        loadRemoteMedia()
    }

    private fun refreshMedia() {
        allThumbnails = emptyList()
        currentPage = 1
        hasMore = true
        loadRemoteMedia()
    }

    private fun updateUI() {
        if (allThumbnails.isEmpty()) {
            emptyStateText.text = "No remote media found"
            emptyStateText.visibility = View.VISIBLE
            recyclerView.visibility = View.GONE
        } else {
            emptyStateText.visibility = View.GONE
            recyclerView.visibility = View.VISIBLE

            val adapter = RemoteMediaAdapter(requireContext(), allThumbnails)
            recyclerView.adapter = adapter
        }
    }

    companion object {
        fun newInstance() = RemoteMediaFragmentNative()
    }
}
EOF

# 4. Create layout file for native remote media fragment
echo "Creating fragment_remote_media_native.xml..."
cat > "$LAYOUT_DIR/fragment_remote_media_native.xml" << 'EOF'
<?xml version="1.0" encoding="utf-8"?>
<androidx.coordinatorlayout.widget.CoordinatorLayout
    xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:app="http://schemas.android.com/apk/res-auto"
    android:layout_width="match_parent"
    android:layout_height="match_parent">

    <androidx.swiperefreshlayout.widget.SwipeRefreshLayout
        android:id="@+id/remoteMediaSwipeRefresh"
        android:layout_width="match_parent"
        android:layout_height="match_parent">

        <androidx.recyclerview.widget.RecyclerView
            android:id="@+id/remoteMediaRecyclerView"
            android:layout_width="match_parent"
            android:layout_height="match_parent"
            android:padding="4dp"
            android:clipToPadding="false" />

    </androidx.swiperefreshlayout.widget.SwipeRefreshLayout>

    <ProgressBar
        android:id="@+id/remoteMediaProgressBar"
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"
        android:layout_gravity="center"
        android:visibility="gone" />

    <TextView
        android:id="@+id/remoteMediaEmptyState"
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"
        android:layout_gravity="center"
        android:text="No remote media found"
        android:textSize="16sp"
        android:textColor="?android:textColorSecondary"
        android:visibility="gone" />

    <com.google.android.material.floatingactionbutton.FloatingActionButton
        android:id="@+id/remoteRefreshFab"
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"
        android:layout_gravity="bottom|end"
        android:layout_margin="16dp"
        android:src="@drawable/ic_refresh"
        app:tint="@android:color/white" />

</androidx.coordinatorlayout.widget.CoordinatorLayout>
EOF

# 5. Create placeholder drawables if they don't exist
echo "Creating placeholder drawable resources..."
mkdir -p "$RES_DIR/drawable"

cat > "$RES_DIR/drawable/ic_image_placeholder.xml" << 'EOF'
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="24dp"
    android:height="24dp"
    android:viewportWidth="24"
    android:viewportHeight="24">
    <path
        android:fillColor="?android:textColorSecondary"
        android:pathData="M19,5v14H5V5H19M19,3H5C3.9,3 3,3.9 3,5v14c0,1.1 0.9,2 2,2h14c1.1,0 2,-0.9 2,-2V5C21,3.9 20.1,3 19,3L19,3z"/>
    <path
        android:fillColor="?android:textColorSecondary"
        android:pathData="M14.14,11.86l-3,3.87L9,13.14L6,17h12L14.14,11.86z"/>
</vector>
EOF

cat > "$RES_DIR/drawable/ic_broken_image.xml" << 'EOF'
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="24dp"
    android:height="24dp"
    android:viewportWidth="24"
    android:viewportHeight="24">
    <path
        android:fillColor="#FF6B6B"
        android:pathData="M21,5v6.59l-3,-3.01 -4,4.01 -4,-4 -4,4 -3,-3.01L3,5c0,-1.1 0.9,-2 2,-2h14c1.1,0 2,0.9 2,2zM18,11.42l3,3.01L21,19c0,1.1 -0.9,2 -2,2L5,21c-1.1,0 -2,-0.9 -2,-2v-6.58l3,2.99 4,-4 4,4 4,-3.99z"/>
</vector>
EOF

cat > "$RES_DIR/drawable/ic_refresh.xml" << 'EOF'
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="24dp"
    android:height="24dp"
    android:viewportWidth="24"
    android:viewportHeight="24">
    <path
        android:fillColor="@android:color/white"
        android:pathData="M17.65,6.35C16.2,4.9 14.21,4 12,4c-4.42,0 -7.99,3.58 -7.99,8s3.57,8 7.99,8c3.73,0 6.84,-2.55 7.73,-6h-2.08c-0.82,2.33 -3.04,4 -5.65,4 -3.31,0 -6,-2.69 -6,-6s2.69,-6 6,-6c1.66,0 3.14,0.69 4.22,1.78L13,11h7V4l-2.35,2.35z"/>
</vector>
EOF

echo ""
echo "=========================================="
echo "Native Remote Gallery Generation Complete!"
echo "=========================================="
echo ""
echo "Generated files:"
echo "  1. $KOTLIN_DIR/repository/RemoteMediaRepository.kt"
echo "  2. $KOTLIN_DIR/RemoteMediaAdapter.kt"
echo "  3. $KOTLIN_DIR/fragments/RemoteMediaFragmentNative.kt"
echo "  4. $LAYOUT_DIR/fragment_remote_media_native.xml"
echo "  5. Drawable resources (ic_image_placeholder, ic_broken_image, ic_refresh)"
echo ""
echo "Next steps:"
echo "  1. Update MainActivity.kt to use RemoteMediaFragmentNative instead of RemoteMediaFragment"
echo "  2. Update ImagePreviewActivity.kt to handle IS_REMOTE flag and load images by hash"
echo "  3. Verify your backend API has a '/api/thumbnails' endpoint that returns paginated results"
echo "  4. Test the implementation and adjust API endpoint URL if needed"
echo ""
echo "To use the native remote gallery, replace in MainActivity.kt:"
echo "  FROM: RemoteMediaFragment.newInstance()"
echo "  TO:   RemoteMediaFragmentNative.newInstance()"
echo ""
EOF

chmod +x /Users/ldr/work/reminisce/android_app/generate_native_remote_gallery.sh