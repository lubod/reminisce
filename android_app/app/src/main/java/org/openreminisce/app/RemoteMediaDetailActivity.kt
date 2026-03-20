package org.openreminisce.app

import android.net.Uri
import android.os.Bundle
import android.util.Log
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import android.widget.Toast
import androidx.activity.viewModels
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import androidx.media3.ui.PlayerView
import com.davemorrissey.labs.subscaleview.ImageSource
import com.davemorrissey.labs.subscaleview.SubsamplingScaleImageView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.Request
import org.openreminisce.app.fragments.MediaInfoBottomSheetFragment
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.AuthenticatedHttpClient
import org.openreminisce.app.util.MediaSessionHolder
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.viewmodel.RemoteMediaDetailViewModel
import java.io.File
import java.io.FileOutputStream
import kotlin.math.abs

@UnstableApi
class RemoteMediaDetailActivity : AppCompatActivity() {

    companion object {
        private const val TAG = "RemoteMediaDetailActivity"
        const val EXTRA_HASH = "IMAGE_HASH"
        const val EXTRA_POSITION = "POSITION"
    }

    private lateinit var imagePreview: SubsamplingScaleImageView
    private lateinit var playerView: PlayerView
    private lateinit var singleViewContainer: FrameLayout
    private lateinit var compareContainer: LinearLayout
    private lateinit var compareImageLeft: SubsamplingScaleImageView
    private lateinit var compareVideoLeft: PlayerView
    private lateinit var compareImageRight: SubsamplingScaleImageView
    private lateinit var compareVideoRight: PlayerView
    private lateinit var loadingSpinner: ProgressBar
    private lateinit var backButton: ImageButton
    private lateinit var compareButton: ImageButton
    private lateinit var deleteButton: ImageButton
    private lateinit var starButton: ImageButton
    private lateinit var infoButton: ImageButton
    private lateinit var prevButton: ImageButton
    private lateinit var nextButton: ImageButton
    private lateinit var counterText: TextView
    private lateinit var gestureDetector: GestureDetector

    private var exoPlayer: ExoPlayer? = null
    private var exoPlayerLeft: ExoPlayer? = null
    private var exoPlayerRight: ExoPlayer? = null
    private var currentTempFile: File? = null
    private var compareTempFileLeft: File? = null
    private var compareTempFileRight: File? = null
    private var isCompareMode: Boolean = false
    private var currentPosition: Int = 0
    private val hashes get() = MediaSessionHolder.hashes
    private val imageInfos get() = MediaSessionHolder.imageInfos

    private val viewModel: RemoteMediaDetailViewModel by viewModels {
        RemoteMediaDetailViewModel.factory(applicationContext)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_remote_media_detail)

        imagePreview = findViewById(R.id.imagePreview)
        playerView = findViewById(R.id.playerView)
        singleViewContainer = findViewById(R.id.singleViewContainer)
        compareContainer = findViewById(R.id.compareContainer)
        compareImageLeft = findViewById(R.id.compareImageLeft)
        compareVideoLeft = findViewById(R.id.compareVideoLeft)
        compareImageRight = findViewById(R.id.compareImageRight)
        compareVideoRight = findViewById(R.id.compareVideoRight)
        loadingSpinner = findViewById(R.id.loadingSpinner)
        backButton = findViewById(R.id.backButton)
        compareButton = findViewById(R.id.compareButton)
        deleteButton = findViewById(R.id.deleteButton)
        starButton = findViewById(R.id.starButton)
        infoButton = findViewById(R.id.infoButton)
        prevButton = findViewById(R.id.prevButton)
        nextButton = findViewById(R.id.nextButton)
        counterText = findViewById(R.id.counterText)

        currentPosition = intent.getIntExtra(EXTRA_POSITION, 0)

        backButton.setOnClickListener { finish() }
        compareButton.setOnClickListener { toggleCompareMode() }
        deleteButton.setOnClickListener { confirmAndDelete() }
        starButton.setOnClickListener { onStarClicked() }
        infoButton.setOnClickListener { openInfoSheet() }
        prevButton.setOnClickListener { showPrevious() }
        nextButton.setOnClickListener { showNext() }

        val density = resources.displayMetrics.density
        gestureDetector = GestureDetector(this, object : GestureDetector.SimpleOnGestureListener() {
            private val SWIPE_THRESHOLD = (80 * density).toInt()
            private val SWIPE_VELOCITY_THRESHOLD = (100 * density).toInt()

            override fun onFling(e1: MotionEvent?, e2: MotionEvent, velocityX: Float, velocityY: Float): Boolean {
                if (e1 == null) return false
                if (isCompareMode) return false
                val diffX = e2.x - e1.x
                val diffY = e2.y - e1.y
                if (abs(diffX) > abs(diffY) &&
                    abs(diffX) > SWIPE_THRESHOLD &&
                    abs(velocityX) > SWIPE_VELOCITY_THRESHOLD
                ) {
                    if (diffX > 0) showPrevious() else showNext()
                    return true
                }
                return false
            }

            override fun onSingleTapConfirmed(e: MotionEvent): Boolean {
                val isVideo = currentMediaType() == "video"
                if (isVideo) {
                    exoPlayer?.let { if (it.isPlaying) it.pause() else it.play() }
                }
                return true
            }
        })

        imagePreview.setOnTouchListener { view, event ->
            val handled = gestureDetector.onTouchEvent(event)
            if (!handled) view.onTouchEvent(event) else handled
        }
        playerView.setOnTouchListener { _, event ->
            gestureDetector.onTouchEvent(event)
            true
        }

        observeViewModel()
        loadCurrentMedia()
    }

    private fun observeViewModel() {
        lifecycleScope.launch {
            viewModel.isStarred.collect { starred ->
                starButton.setImageResource(
                    if (starred) R.drawable.ic_star else R.drawable.ic_star_outline
                )
            }
        }
        lifecycleScope.launch {
            viewModel.error.collect { msg ->
                Toast.makeText(this@RemoteMediaDetailActivity, msg, Toast.LENGTH_SHORT).show()
            }
        }
        lifecycleScope.launch {
            viewModel.starToggleResult.collect { (hash, starred) ->
                MediaSessionHolder.starredUpdates[hash] = starred
            }
        }
        lifecycleScope.launch {
            viewModel.deleteResult.collect { result ->
                result.fold(
                    onSuccess = {
                        // Record deletion so the grid can remove it on return
                        val deletedHash = if (currentPosition < hashes.size) hashes[currentPosition] else null
                        deletedHash?.let { MediaSessionHolder.deletedHashes.add(it) }
                        // Remove from session and navigate
                        MediaSessionHolder.hashes = MediaSessionHolder.hashes.toMutableList().also {
                            it.removeAt(currentPosition)
                        }
                        MediaSessionHolder.imageInfos = MediaSessionHolder.imageInfos.toMutableList().also {
                            it.removeAt(currentPosition)
                        }
                        if (hashes.isEmpty()) {
                            finish()
                        } else {
                            if (currentPosition >= hashes.size) currentPosition = hashes.size - 1
                            loadCurrentMedia()
                        }
                    },
                    onFailure = { /* error already shown via _error flow */ }
                )
            }
        }
    }

    private fun currentHash(): String = if (hashes.isNotEmpty() && currentPosition < hashes.size)
        hashes[currentPosition] else ""

    private fun currentMediaType(): String =
        if (imageInfos.isNotEmpty() && currentPosition < imageInfos.size)
            imageInfos[currentPosition].mediaType else "image"

    private fun loadCurrentMedia() {
        val hash = currentHash()
        if (hash.isEmpty()) { finish(); return }

        // Exit compare mode when navigating
        if (isCompareMode) {
            isCompareMode = false
            syncCompareVisibility()
        }

        updateNavigationUI()
        viewModel.loadMetadata(hash)
        loadMedia(hash, currentMediaType())
    }

    private fun loadMedia(hash: String, mediaType: String) {
        loadingSpinner.visibility = View.VISIBLE
        releasePlayer()

        val baseUrl = PreferenceHelper.getServerUrl(this)
        val isVideo = mediaType == "video"
        val endpoint = if (isVideo) "video" else "image"
        val mediaUrl = "$baseUrl/api/$endpoint/$hash"

        if (isVideo) {
            imagePreview.visibility = View.GONE
            playerView.visibility = View.VISIBLE
            playVideo(mediaUrl)
        } else {
            playerView.visibility = View.GONE
            imagePreview.visibility = View.VISIBLE
            downloadImage(mediaUrl)
        }
    }

    private fun playVideo(mediaUrl: String) {
        lifecycleScope.launch {
            val token = withContext(Dispatchers.IO) {
                AuthHelper.getValidToken(this@RemoteMediaDetailActivity)
            } ?: return@launch

            val okHttpClient = AuthenticatedHttpClient.getClientWithTimeouts(this@RemoteMediaDetailActivity, 30, 300)
            val dataSourceFactory = OkHttpDataSource.Factory(okHttpClient)
                .setDefaultRequestProperties(mapOf("Authorization" to "Bearer $token"))

            exoPlayer = ExoPlayer.Builder(this@RemoteMediaDetailActivity).build().also { player ->
                playerView.player = player
                val source = ProgressiveMediaSource.Factory(dataSourceFactory)
                    .createMediaSource(androidx.media3.common.MediaItem.fromUri(Uri.parse(mediaUrl)))
                player.setMediaSource(source)
                player.prepare()
                player.playWhenReady = true
                player.addListener(object : Player.Listener {
                    override fun onPlaybackStateChanged(state: Int) {
                        if (state == Player.STATE_READY) loadingSpinner.visibility = View.GONE
                    }

                    override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
                        loadingSpinner.visibility = View.GONE
                        Log.e(TAG, "Video error: ${error.message}")
                    }
                })
            }
        }
    }

    private fun downloadImage(mediaUrl: String) {
        lifecycleScope.launch {
            try {
                val token = withContext(Dispatchers.IO) {
                    AuthHelper.getValidToken(this@RemoteMediaDetailActivity)
                }
                if (token == null) {
                    loadingSpinner.visibility = View.GONE
                    return@launch
                }

                val tempFile = withContext(Dispatchers.IO) {
                    val client = AuthenticatedHttpClient.getClient(this@RemoteMediaDetailActivity)
                    val request = Request.Builder()
                        .url(mediaUrl)
                        .addHeader("Authorization", "Bearer $token")
                        .build()
                    val response = client.newCall(request).execute()
                    if (!response.isSuccessful) return@withContext null
                    val file = File.createTempFile("rmd_", ".jpg", cacheDir)
                    try {
                        response.body?.byteStream()?.use { input ->
                            FileOutputStream(file).use { output -> input.copyTo(output) }
                        }
                        file
                    } catch (e: Exception) {
                        file.delete()
                        throw e
                    }
                }

                currentTempFile?.delete()
                currentTempFile = tempFile
                if (tempFile != null) {
                    imagePreview.setImage(ImageSource.uri(tempFile.absolutePath))
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error downloading image", e)
            } finally {
                loadingSpinner.visibility = View.GONE
            }
        }
    }

    // ── Compare mode ─────────────────────────────────────────────────────────

    private fun toggleCompareMode() {
        if (isCompareMode) {
            isCompareMode = false
            releaseComparePlayers()
            syncCompareVisibility()
        } else {
            val nextPos = currentPosition + 1
            if (nextPos >= hashes.size) {
                Toast.makeText(this, "No next item to compare", Toast.LENGTH_SHORT).show()
                return
            }
            isCompareMode = true
            syncCompareVisibility()
            loadCompareMedia(currentPosition, isLeft = true)
            loadCompareMedia(nextPos, isLeft = false)
        }
    }

    private fun syncCompareVisibility() {
        if (isCompareMode) {
            singleViewContainer.visibility = View.GONE
            compareContainer.visibility = View.VISIBLE
        } else {
            singleViewContainer.visibility = View.VISIBLE
            compareContainer.visibility = View.GONE
        }
    }

    private fun loadCompareMedia(position: Int, isLeft: Boolean) {
        val hash = if (position < hashes.size) hashes[position] else return
        val mediaType = if (position < imageInfos.size) imageInfos[position].mediaType else "image"
        val baseUrl = PreferenceHelper.getServerUrl(this)
        val isVideo = mediaType == "video"
        val endpoint = if (isVideo) "video" else "image"
        val mediaUrl = "$baseUrl/api/$endpoint/$hash"

        val imageView = if (isLeft) compareImageLeft else compareImageRight
        val videoView = if (isLeft) compareVideoLeft else compareVideoRight

        if (isVideo) {
            imageView.visibility = View.GONE
            videoView.visibility = View.VISIBLE
            loadCompareVideo(mediaUrl, videoView, isLeft)
        } else {
            videoView.visibility = View.GONE
            imageView.visibility = View.VISIBLE
            loadCompareImage(mediaUrl, imageView, isLeft)
        }
    }

    private fun loadCompareVideo(mediaUrl: String, videoView: PlayerView, isLeft: Boolean) {
        lifecycleScope.launch {
            val token = withContext(Dispatchers.IO) {
                AuthHelper.getValidToken(this@RemoteMediaDetailActivity)
            } ?: return@launch

            val okHttpClient = AuthenticatedHttpClient.getClientWithTimeouts(this@RemoteMediaDetailActivity, 30, 300)
            val dataSourceFactory = OkHttpDataSource.Factory(okHttpClient)
                .setDefaultRequestProperties(mapOf("Authorization" to "Bearer $token"))

            val player = ExoPlayer.Builder(this@RemoteMediaDetailActivity).build().also { p ->
                videoView.player = p
                val source = ProgressiveMediaSource.Factory(dataSourceFactory)
                    .createMediaSource(androidx.media3.common.MediaItem.fromUri(Uri.parse(mediaUrl)))
                p.setMediaSource(source)
                p.prepare()
                p.playWhenReady = false
            }
            if (isLeft) exoPlayerLeft = player else exoPlayerRight = player
        }
    }

    private fun loadCompareImage(mediaUrl: String, imageView: SubsamplingScaleImageView, isLeft: Boolean) {
        lifecycleScope.launch {
            try {
                val token = withContext(Dispatchers.IO) {
                    AuthHelper.getValidToken(this@RemoteMediaDetailActivity)
                } ?: return@launch

                val tempFile = withContext(Dispatchers.IO) {
                    val client = AuthenticatedHttpClient.getClient(this@RemoteMediaDetailActivity)
                    val request = Request.Builder()
                        .url(mediaUrl)
                        .addHeader("Authorization", "Bearer $token")
                        .build()
                    val response = client.newCall(request).execute()
                    if (!response.isSuccessful) return@withContext null
                    val file = File.createTempFile("cmp_${if (isLeft) "L" else "R"}_", ".jpg", cacheDir)
                    try {
                        response.body?.byteStream()?.use { input ->
                            FileOutputStream(file).use { output -> input.copyTo(output) }
                        }
                        file
                    } catch (e: Exception) {
                        file.delete()
                        throw e
                    }
                }

                if (tempFile != null) {
                    if (isLeft) {
                        compareTempFileLeft?.delete()
                        compareTempFileLeft = tempFile
                    } else {
                        compareTempFileRight?.delete()
                        compareTempFileRight = tempFile
                    }
                    imageView.setImage(ImageSource.uri(tempFile.absolutePath))
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error downloading compare image", e)
            }
        }
    }

    private fun releaseComparePlayers() {
        exoPlayerLeft?.release()
        exoPlayerLeft = null
        exoPlayerRight?.release()
        exoPlayerRight = null
        compareTempFileLeft?.delete()
        compareTempFileLeft = null
        compareTempFileRight?.delete()
        compareTempFileRight = null
    }

    // ── Delete ────────────────────────────────────────────────────────────────

    private fun confirmAndDelete() {
        val hash = currentHash()
        if (hash.isEmpty()) return
        AlertDialog.Builder(this)
            .setTitle("Delete media")
            .setMessage("Permanently delete this item? This cannot be undone.")
            .setPositiveButton("Delete") { _, _ ->
                viewModel.deleteMedia(hash, currentMediaType())
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    private fun showNext() {
        if (currentPosition >= hashes.size - 1) return
        currentPosition++
        loadCurrentMedia()
    }

    private fun showPrevious() {
        if (currentPosition <= 0) return
        currentPosition--
        loadCurrentMedia()
    }

    private fun updateNavigationUI() {
        counterText.text = "${currentPosition + 1} / ${hashes.size}"
        prevButton.isEnabled = currentPosition > 0
        prevButton.alpha = if (currentPosition > 0) 1f else 0.4f
        nextButton.isEnabled = currentPosition < hashes.size - 1
        nextButton.alpha = if (currentPosition < hashes.size - 1) 1f else 0.4f
    }

    private fun onStarClicked() {
        val hash = currentHash()
        if (hash.isEmpty()) return
        viewModel.toggleStar(hash, currentMediaType())
    }

    private fun openInfoSheet() {
        val hash = currentHash()
        if (hash.isEmpty()) return
        val mediaType = currentMediaType()
        viewModel.loadAllLabels()
        viewModel.loadMediaLabels(hash, mediaType)
        MediaInfoBottomSheetFragment.newInstance(hash, mediaType)
            .show(supportFragmentManager, "media_info")
    }

    private fun releasePlayer() {
        exoPlayer?.release()
        exoPlayer = null
    }

    override fun onPause() {
        super.onPause()
        exoPlayer?.pause()
        exoPlayerLeft?.pause()
        exoPlayerRight?.pause()
    }

    override fun onDestroy() {
        super.onDestroy()
        releasePlayer()
        releaseComparePlayers()
        currentTempFile?.delete()
        currentTempFile = null
        MediaSessionHolder.clear()
    }
}
