package org.openreminisce.app

import android.net.Uri
import android.os.Bundle
import android.util.Log
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.View
import android.widget.ProgressBar
import androidx.appcompat.app.AppCompatActivity
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.util.SecureStorageHelper
import kotlin.math.abs
import android.widget.ImageButton
import android.widget.TextView
import android.widget.LinearLayout
import org.openreminisce.app.model.ImageInfo
import okhttp3.Call
import okhttp3.Callback
import okhttp3.Request
import okhttp3.Response
import java.io.IOException
import android.graphics.Bitmap
import java.io.File
import java.io.FileOutputStream
import com.davemorrissey.labs.subscaleview.SubsamplingScaleImageView
import com.davemorrissey.labs.subscaleview.ImageSource
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import androidx.media3.ui.PlayerView
import com.bumptech.glide.Glide
import androidx.media3.datasource.DefaultDataSource
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

class ImagePreviewActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "ImagePreviewActivity"
    }

    private lateinit var imagePreview: SubsamplingScaleImageView
    private lateinit var playerView: PlayerView
    private var exoPlayer: ExoPlayer? = null
    private lateinit var loadingSpinner: ProgressBar
    private lateinit var gestureDetector: GestureDetector
    private lateinit var prevButton: ImageButton
    private lateinit var nextButton: ImageButton
    private lateinit var backToMainButton: ImageButton
    private lateinit var imageNameText: TextView
    private lateinit var imageDateText: TextView
    private lateinit var imagePlaceText: TextView
    private lateinit var infoOverlay: LinearLayout

    private var imageHashes: ArrayList<String> = arrayListOf()
    private var currentPosition: Int = 0
    private var isVideo: Boolean = false
    private var isLocalMedia: Boolean = false // Track if viewing local media
    private var imageInfos: ArrayList<ImageInfo> = arrayListOf() // Store image info for display
    private var currentFileName: String? = null // Store filename from Content-Disposition header
    private var isLocationExpanded: Boolean = false // Track if location text is expanded
    private var fullLocationText: String? = null // Store full location text

    @UnstableApi
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_image_preview)

        imagePreview = findViewById(R.id.imagePreview)
        playerView = findViewById(R.id.playerView)
        loadingSpinner = findViewById(R.id.loadingSpinner)
        prevButton = findViewById(R.id.prevButton)
        nextButton = findViewById(R.id.nextButton)
        backToMainButton = findViewById(R.id.backToMainButton)
        imageNameText = findViewById(R.id.imageNameText)
        imageDateText = findViewById(R.id.imageDateText)
        imagePlaceText = findViewById(R.id.imagePlaceText)
        infoOverlay = findViewById(R.id.infoOverlay)

        // Get the image hash and list from intent
        // Support both old and new intent extra names for compatibility
        val imageHash = intent.getStringExtra("imageHash") ?: intent.getStringExtra("IMAGE_HASH")
        imageHashes = intent.getStringArrayListExtra("imageHashes")
            ?: intent.getStringArrayListExtra("ALL_HASHES")
            ?: arrayListOf()
        currentPosition = intent.getIntExtra("position", 0) ?: intent.getIntExtra("POSITION", 0)
        isVideo = intent.getBooleanExtra("isVideo", false)

        // Handle both isLocalMedia and IS_REMOTE flags (IS_REMOTE is opposite of isLocalMedia)
        val isRemote = intent.getBooleanExtra("IS_REMOTE", false)
        isLocalMedia = intent.getBooleanExtra("isLocalMedia", false) && !isRemote

        // Try to get the image info list from intent
        imageInfos = if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
            @Suppress("UNCHECKED_CAST")
            intent.getSerializableExtra("imageInfos", ArrayList::class.java) as? ArrayList<ImageInfo> ?: arrayListOf()
        } else {
            @Suppress("DEPRECATION", "UNCHECKED_CAST")
            intent.getSerializableExtra("imageInfos") as? ArrayList<ImageInfo> ?: arrayListOf()
        }

        if (imageHash == null) {
            finish()
            return
        }

        // Set up button click listeners
        prevButton.setOnClickListener {
            showPreviousMedia()
        }

        nextButton.setOnClickListener {
            showNextMedia()
        }

        backToMainButton.setOnClickListener {
            finish() // Go back to the main activity
        }

        // Manage button visibility based on whether we have a list of images/videos
        updateNavigationButtons()

        // Display current image info
        updateImageInfo()

        // Setup gesture detector for swipe
        gestureDetector = GestureDetector(this, object : GestureDetector.SimpleOnGestureListener() {
            private val SWIPE_THRESHOLD = 100
            private val SWIPE_VELOCITY_THRESHOLD = 100

            override fun onFling(
                e1: MotionEvent?,
                e2: MotionEvent,
                velocityX: Float,
                velocityY: Float
            ): Boolean {
                if (e1 == null) return false

                val diffX = e2.x - e1.x
                val diffY = e2.y - e1.y

                if (abs(diffX) > abs(diffY)) {
                    if (abs(diffX) > SWIPE_THRESHOLD && abs(velocityX) > SWIPE_VELOCITY_THRESHOLD) {
                        if (diffX > 0) {
                            // Swipe right - previous
                            showPreviousMedia()
                        } else {
                            // Swipe left - next
                            showNextMedia()
                        }
                        return true
                    }
                }
                return false
            }

            override fun onSingleTapConfirmed(e: MotionEvent): Boolean {
                if (isVideo && exoPlayer?.isPlaying == true) {
                    // Toggle play/pause for video
                    if (exoPlayer?.isPlaying == true) {
                        exoPlayer?.pause()
                    } else {
                        exoPlayer?.play()
                    }
                } else {
                    // Close preview on tap for images
                    finish()
                }
                return true
            }
        })

        // For imagePreview: Allow SubsamplingScaleImageView to handle zoom gestures
        // Only consume touch events if gesture detector handles them (swipes, taps)
        val imageViewTouchListener = View.OnTouchListener { view, event ->
            // Let gesture detector try to handle the event first
            val handled = gestureDetector.onTouchEvent(event)
            // If gesture detector didn't handle it (no swipe/tap detected),
            // let the SubsamplingScaleImageView handle it for zoom gestures
            if (!handled) {
                view.onTouchEvent(event)
            } else {
                handled
            }
        }

        // For playerView: Consume all events (no zoom needed for video)
        val playerViewTouchListener = View.OnTouchListener { _, event ->
            gestureDetector.onTouchEvent(event)
            true
        }

        imagePreview.setOnTouchListener(imageViewTouchListener)
        playerView.setOnTouchListener(playerViewTouchListener)

        // Load the media
        loadImage(imageHash)
    }

    private fun updateImageInfo() {
        // Reset location expansion state when changing images
        isLocationExpanded = false
        fullLocationText = null

        if (imageInfos.isNotEmpty() && currentPosition >= 0 && currentPosition < imageInfos.size) {
            // Use the image info at current position
            val currentImageInfo = imageInfos[currentPosition]

            // Priority: Display filename from Content-Disposition if available, otherwise use image ID
            val displayName = if (!currentFileName.isNullOrEmpty()) {
                currentFileName!!
            } else if (currentImageInfo.id.length > 20) {
                "..." + currentImageInfo.id.substring(currentImageInfo.id.length - 17)
            } else {
                currentImageInfo.id
            }

            // Format the date and time: "1 Jan 2025, 8:55:10"
            val dateTimeFormat = java.text.SimpleDateFormat("d MMM yyyy, H:mm:ss", java.util.Locale.getDefault())
            val formattedDateTime = dateTimeFormat.format(currentImageInfo.date)

            imageNameText.text = displayName
            imageDateText.text = "Created: $formattedDateTime"

            // Display place information if available
            if (!currentImageInfo.place.isNullOrEmpty()) {
                fullLocationText = currentImageInfo.place
                updateLocationDisplay()
                imagePlaceText.visibility = View.VISIBLE
            } else {
                imagePlaceText.visibility = View.GONE
            }

            infoOverlay.visibility = View.VISIBLE
        } else {
            // If we don't have image info, try to get it from imageHashes
            // This fallback is for cases where older code doesn't pass ImageInfo
            if (imageHashes.isNotEmpty() && currentPosition >= 0 && currentPosition < imageHashes.size) {
                val currentHash = imageHashes[currentPosition]

                // Priority: Display filename from Content-Disposition if available, otherwise use hash
                val displayName = if (!currentFileName.isNullOrEmpty()) {
                    currentFileName!!
                } else if (currentHash.length > 20) {
                    "..." + currentHash.substring(currentHash.length - 17)
                } else {
                    currentHash
                }

                imageNameText.text = displayName
                imageDateText.text = "Date unknown"
                imagePlaceText.visibility = View.GONE

                infoOverlay.visibility = View.VISIBLE
            } else {
                infoOverlay.visibility = View.GONE
            }
        }
    }

    private fun updateLocationDisplay() {
        val location = fullLocationText ?: return

        if (location.length > 40 && !isLocationExpanded) {
            // Truncate to 40 chars and add ellipsis
            imagePlaceText.text = location.take(40) + "..."
            imagePlaceText.maxLines = 1
        } else {
            // Show full text
            imagePlaceText.text = location
            imagePlaceText.maxLines = Int.MAX_VALUE
        }

        // Set up click listener to toggle expansion
        imagePlaceText.setOnClickListener {
            if (location.length > 40) {
                isLocationExpanded = !isLocationExpanded
                updateLocationDisplay()
            }
        }
    }

    @UnstableApi
    private fun showNextMedia() {
        if (imageHashes.isEmpty()) return

        currentPosition++
        if (currentPosition >= imageInfos.size - 1) {
            return
        }

        loadImage(imageHashes[currentPosition])
        updateImageInfo()
    }

    @UnstableApi
    private fun showPreviousMedia() {
        if (imageHashes.isEmpty()) return

        currentPosition--
        if (currentPosition <= 0) {
            return
        }

        loadImage(imageHashes[currentPosition])
        updateImageInfo()
    }

    private fun updateNavigationButtons() {
        // Hide navigation buttons if we don't have a list of images/videos
        if (imageHashes.size <= 1) {
            prevButton.visibility = View.GONE
            nextButton.visibility = View.GONE
        } else {
            prevButton.visibility = View.VISIBLE
            nextButton.visibility = View.VISIBLE

            // Disable prev button if at first item
            prevButton.isEnabled = currentPosition > 0
            // Disable next button if at last item
            nextButton.isEnabled = currentPosition < imageHashes.size - 1
        }
    }

    @UnstableApi
    private fun loadImage(imageHash: String) {
        loadingSpinner.visibility = View.VISIBLE

        // Clear previous filename
        currentFileName = null

        // Release existing player if any
        releasePlayer()

        // Handle local media differently
        if (isLocalMedia) {
            loadLocalMedia(imageHash)
            return
        }

        Thread {
            try {
                // Get authentication token
                val token = AuthHelper.getValidToken(this)
                val deviceId = AuthHelper.getDeviceId(this)

                if (token.isNullOrEmpty()) {
                    Log.e(TAG, "Failed to get auth token")
                    return@Thread
                }

                // Build the full image/video URL based on type
                val baseUrl = PreferenceHelper.getServerUrl(this)
                val endpoint = if (isVideo) "video" else "image"
                val mediaUrl = "$baseUrl/api/$endpoint/$imageHash"

                runOnUiThread {
                    if (isVideo) {
                        // Show video, hide image
                        imagePreview.visibility = View.GONE
                        playerView.visibility = View.VISIBLE

                        // First, fetch the filename from Content-Disposition header using HEAD request
                        fetchVideoFilename(mediaUrl, token, deviceId)

                        // Initialize ExoPlayer
                        exoPlayer = ExoPlayer.Builder(this).build().also { player ->
                            playerView.player = player

                            // Use our OkHttpClient with SSL configuration and authentication
                            val okHttpClient = org.openreminisce.app.util.AuthenticatedHttpClient
                                .getClientWithTimeouts(this, 30, 300)

                            // Create OkHttp data source factory with our configured client
                            val dataSourceFactory = OkHttpDataSource.Factory(okHttpClient)
                                .setDefaultRequestProperties(mapOf(
                                    "Authorization" to "Bearer $token"
                                ))

                            // Create media source
                            val mediaSource = ProgressiveMediaSource.Factory(dataSourceFactory)
                                .createMediaSource(MediaItem.fromUri(Uri.parse(mediaUrl)))

                            // Set up player
                            player.setMediaSource(mediaSource)
                            player.prepare()
                            player.playWhenReady = true

                            // Add listener for player events
                            player.addListener(object : Player.Listener {
                                override fun onPlaybackStateChanged(playbackState: Int) {
                                    when (playbackState) {
                                        Player.STATE_READY -> {
                                            loadingSpinner.visibility = View.GONE
                                            Log.d("ImagePreview", "Video ready to play")
                                        }
                                        Player.STATE_ENDED -> {
                                            Log.d("ImagePreview", "Video playback completed")
                                        }
                                    }
                                }

                                override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
                                    loadingSpinner.visibility = View.GONE
                                    Log.e("ImagePreview", "Video error: ${error.message}", error)
                                }
                            })
                        }

                    } else {
                        // Show image, hide video
                        playerView.visibility = View.GONE
                        imagePreview.visibility = View.VISIBLE

                        // Download image and set it to SubsamplingScaleImageView
                        downloadImageAndSetToView(mediaUrl, token, deviceId)
                    }

                    // Update navigation button states after loading media
                    updateNavigationButtons()
                    // Update image info display
                    updateImageInfo()
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error loading media", e)
                runOnUiThread {
                    loadingSpinner.visibility = View.GONE
                }
            }
        }.start()
    }

    @UnstableApi
    private fun loadLocalMedia(mediaUri: String) {
        try {
            val uri = Uri.parse(mediaUri)

            // Get current ImageInfo to determine if it's a video
            val currentInfo = if (imageInfos.isNotEmpty() && currentPosition < imageInfos.size) {
                imageInfos[currentPosition]
            } else {
                null
            }

            // Check if this is a video based on ImageInfo or intent flag
            val isCurrentVideo = currentInfo?.mediaType == "video" || isVideo

            if (isCurrentVideo) {
                // Show video, hide image
                imagePreview.visibility = View.GONE
                playerView.visibility = View.VISIBLE

                // Initialize ExoPlayer for local video
                exoPlayer = ExoPlayer.Builder(this).build().also { player ->
                    playerView.player = player

                    // Create media source for local URI
                    val dataSourceFactory = DefaultDataSource.Factory(this)
                    val mediaSource = ProgressiveMediaSource.Factory(dataSourceFactory)
                        .createMediaSource(MediaItem.fromUri(uri))

                    // Set up player
                    player.setMediaSource(mediaSource)
                    player.prepare()
                    player.playWhenReady = true

                    // Add listener for player events
                    player.addListener(object : Player.Listener {
                        override fun onPlaybackStateChanged(playbackState: Int) {
                            when (playbackState) {
                                Player.STATE_READY -> {
                                    loadingSpinner.visibility = View.GONE
                                    Log.d("ImagePreview", "Local video ready to play")
                                }
                                Player.STATE_ENDED -> {
                                    Log.d("ImagePreview", "Local video playback completed")
                                }
                            }
                        }

                        override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
                            loadingSpinner.visibility = View.GONE
                            Log.e("ImagePreview", "Local video error: ${error.message}", error)
                        }
                    })
                }
            } else {
                // Show image, hide video
                playerView.visibility = View.GONE
                imagePreview.visibility = View.VISIBLE

                // Load local image using Glide and save to temp file for SubsamplingScaleImageView
                Glide.with(this)
                    .asBitmap()
                    .load(uri)
                    .into(object : com.bumptech.glide.request.target.CustomTarget<Bitmap>() {
                        override fun onResourceReady(
                            resource: Bitmap,
                            transition: com.bumptech.glide.request.transition.Transition<in Bitmap>?
                        ) {
                            try {
                                // Save bitmap to temp file
                                val tempFile = File.createTempFile("local_image_", ".jpg", cacheDir)
                                FileOutputStream(tempFile).use { out ->
                                    resource.compress(Bitmap.CompressFormat.JPEG, 100, out)
                                }

                                // Load into SubsamplingScaleImageView
                                imagePreview.setImage(ImageSource.uri(Uri.fromFile(tempFile)))
                                loadingSpinner.visibility = View.GONE
                            } catch (e: Exception) {
                                Log.e("ImagePreview", "Error saving local image to temp file", e)
                                loadingSpinner.visibility = View.GONE
                            }
                        }

                        override fun onLoadCleared(placeholder: android.graphics.drawable.Drawable?) {
                            // Clean up if needed
                        }

                        override fun onLoadFailed(errorDrawable: android.graphics.drawable.Drawable?) {
                            loadingSpinner.visibility = View.GONE
                        }
                    })
            }

            // Update navigation button states
            updateNavigationButtons()
            // Update image info display
            updateImageInfo()

        } catch (e: Exception) {
            Log.e("ImagePreview", "Error loading local media", e)
            loadingSpinner.visibility = View.GONE
        }
    }

    private fun fetchVideoFilename(mediaUrl: String, token: String, deviceId: String) {
        val client = org.openreminisce.app.util.AuthenticatedHttpClient.getClient(this)

        // Use HEAD request to get headers without downloading the video
        val request = Request.Builder()
            .url(mediaUrl)
            .head()
            .addHeader("Authorization", "Bearer $token")
            .build()

        client.newCall(request).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) {
                Log.e("ImagePreview", "Failed to fetch video filename", e)
                // Continue playing video even if we couldn't get the filename
            }

            override fun onResponse(call: Call, response: Response) {
                if (response.isSuccessful) {
                    // Extract filename from Content-Disposition header
                    val contentDisposition = response.header("Content-Disposition")
                    if (contentDisposition != null) {
                        val filenameMatch = Regex("""filename="?([^"]+)"?""").find(contentDisposition)
                        if (filenameMatch != null) {
                            val fullPath = filenameMatch.groupValues[1]
                            // Extract only the filename, removing any path components
                            currentFileName = File(fullPath).name
                            Log.d("ImagePreview", "Extracted video filename from header: $currentFileName")
                            // Update UI with the filename
                            runOnUiThread {
                                updateImageInfo()
                            }
                        }
                    }
                }
            }
        })
    }

    private fun downloadImageAndSetToView(mediaUrl: String, token: String, deviceId: String) {
        val client = org.openreminisce.app.util.AuthenticatedHttpClient.getClient(this)

        val request = Request.Builder()
            .url(mediaUrl)
            .addHeader("Authorization", "Bearer $token")
            .build()

        client.newCall(request).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) {
                runOnUiThread {
                    loadingSpinner.visibility = View.GONE
                    Log.e("ImagePreview", "Failed to download image", e)
                }
            }

            override fun onResponse(call: Call, response: Response) {
                if (!response.isSuccessful) {
                    runOnUiThread {
                        loadingSpinner.visibility = View.GONE
                    }
                    return
                }

                try {
                    // Extract filename from Content-Disposition header
                    val contentDisposition = response.header("Content-Disposition")
                    if (contentDisposition != null) {
                        // Parse Content-Disposition header: attachment; filename="IMG_20231231.jpg"
                        val filenameMatch = Regex("""filename="?([^"]+)"?""").find(contentDisposition)
                        if (filenameMatch != null) {
                            val fullPath = filenameMatch.groupValues[1]
                            // Extract only the filename, removing any path components
                            currentFileName = File(fullPath).name
                            Log.d("ImagePreview", "Extracted filename from header: $currentFileName")
                        }
                    }

                    // Save the response body to a temporary file
                    val responseBody = response.body
                    if (responseBody != null) {
                        val tempFile = File.createTempFile("image_", ".jpg", cacheDir)
                        val outputStream = java.io.FileOutputStream(tempFile)
                        responseBody.byteStream().copyTo(outputStream)
                        outputStream.close()

                        runOnUiThread {
                            // Set the image from the temporary file
                            imagePreview.setImage(ImageSource.uri(tempFile.absolutePath))
                            loadingSpinner.visibility = View.GONE
                            // Update info display with filename
                            updateImageInfo()
                        }
                    } else {
                        runOnUiThread {
                            loadingSpinner.visibility = View.GONE
                        }
                    }
                } catch (e: Exception) {
                    runOnUiThread {
                        loadingSpinner.visibility = View.GONE
                        Log.e("ImagePreview", "Error handling image response", e)
                    }
                }
            }
        })
    }

    private fun releasePlayer() {
        exoPlayer?.release()
        exoPlayer = null
    }

    override fun onPause() {
        super.onPause()
        // Pause video when activity is paused
        exoPlayer?.pause()
    }

    override fun onDestroy() {
        super.onDestroy()
        // Release player resources
        releasePlayer()
    }
}
