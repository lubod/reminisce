package org.openreminisce.app

import android.content.ContentValues
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.MediaStore
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
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.Request
import org.openreminisce.app.fragments.MediaInfoBottomSheetFragment
import org.openreminisce.app.repository.RemoteMediaRepository
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.AuthenticatedHttpClient
import org.openreminisce.app.util.MediaSessionHolder
import org.openreminisce.app.util.PreferenceHelper
import org.openreminisce.app.viewmodel.RemoteMediaDetailViewModel
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.OutputStream
import kotlin.math.abs

@androidx.annotation.OptIn(markerClass = [UnstableApi::class])
class RemoteMediaDetailActivity : AppCompatActivity() {

    companion object {
        private const val TAG = "RemoteMediaDetailActivity"
        const val EXTRA_HASH = "IMAGE_HASH"
        const val EXTRA_POSITION = "POSITION"
        private const val STATE_POSITION = "state_position"
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
    private lateinit var rotateLeftButton: ImageButton
    private lateinit var rotateRightButton: ImageButton
    private lateinit var enhanceButton: ImageButton
    private lateinit var downloadButton: ImageButton
    private lateinit var compareButton: ImageButton
    private lateinit var deleteButton: ImageButton
    private lateinit var starButton: ImageButton
    private lateinit var infoButton: ImageButton
    private lateinit var enhanceActionBar: LinearLayout
    private lateinit var enhanceStatusText: TextView
    private lateinit var enhanceToggleOriginalButton: TextView
    private lateinit var enhanceSaveButton: TextView
    private lateinit var enhanceDiscardButton: TextView
    private lateinit var prevButton: ImageButton
    private lateinit var nextButton: ImageButton
    private lateinit var counterText: TextView
    private lateinit var gestureDetector: GestureDetector

    private var exoPlayer: ExoPlayer? = null
    private var exoPlayerLeft: ExoPlayer? = null
    private var exoPlayerRight: ExoPlayer? = null
    private var currentTempFile: File? = null
    private var enhancedTempFile: File? = null
    private var enhancedBytes: ByteArray? = null
    private var showingEnhanced: Boolean = false
    private var compareTempFileLeft: File? = null
    private var compareTempFileRight: File? = null
    private var isCompareMode: Boolean = false
    private var isRotating: Boolean = false
    private var isEnhancing: Boolean = false
    private var currentPosition: Int = 0
    private var currentMediaJob: Job? = null
    private val hashes get() = MediaSessionHolder.hashes
    private val imageInfos get() = MediaSessionHolder.imageInfos

    private val viewModel: RemoteMediaDetailViewModel by viewModels {
        RemoteMediaDetailViewModel.factory(applicationContext)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_remote_media_detail)

        supportActionBar?.hide()
        window.statusBarColor = android.graphics.Color.BLACK

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
        rotateLeftButton = findViewById(R.id.rotateLeftButton)
        rotateRightButton = findViewById(R.id.rotateRightButton)
        enhanceButton = findViewById(R.id.enhanceButton)
        downloadButton = findViewById(R.id.downloadButton)
        compareButton = findViewById(R.id.compareButton)
        deleteButton = findViewById(R.id.deleteButton)
        starButton = findViewById(R.id.starButton)
        infoButton = findViewById(R.id.infoButton)
        enhanceActionBar = findViewById(R.id.enhanceActionBar)
        enhanceStatusText = findViewById(R.id.enhanceStatusText)
        enhanceToggleOriginalButton = findViewById(R.id.enhanceToggleOriginalButton)
        enhanceSaveButton = findViewById(R.id.enhanceSaveButton)
        enhanceDiscardButton = findViewById(R.id.enhanceDiscardButton)
        prevButton = findViewById(R.id.prevButton)
        nextButton = findViewById(R.id.nextButton)
        counterText = findViewById(R.id.counterText)

        imagePreview.orientation = SubsamplingScaleImageView.ORIENTATION_USE_EXIF
        compareImageLeft.orientation = SubsamplingScaleImageView.ORIENTATION_USE_EXIF
        compareImageRight.orientation = SubsamplingScaleImageView.ORIENTATION_USE_EXIF

        currentPosition = savedInstanceState?.getInt(STATE_POSITION)
            ?: intent.getIntExtra(EXTRA_POSITION, 0)
        if (hashes.isNotEmpty() && currentPosition >= hashes.size) {
            currentPosition = hashes.size - 1
        }

        backButton.setOnClickListener { finish() }
        rotateLeftButton.setOnClickListener { rotateCurrentImage("ccw") }
        rotateRightButton.setOnClickListener { rotateCurrentImage("cw") }
        enhanceButton.setOnClickListener { showEnhanceModeDialog() }
        downloadButton.setOnClickListener { downloadCurrentMedia() }
        enhanceToggleOriginalButton.setOnClickListener { toggleEnhancedPreview() }
        enhanceSaveButton.setOnClickListener { saveEnhancedCopy() }
        enhanceDiscardButton.setOnClickListener { clearEnhanceState(restoreOriginal = true) }
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

        clearEnhanceState(restoreOriginal = false)

        // Exit compare mode when navigating
        if (isCompareMode) {
            isCompareMode = false
            syncCompareVisibility()
        }

        val isImage = currentMediaType() != "video"
        rotateLeftButton.visibility = if (isImage) View.VISIBLE else View.GONE
        rotateRightButton.visibility = if (isImage) View.VISIBLE else View.GONE
        enhanceButton.visibility = if (isImage) View.VISIBLE else View.GONE

        updateNavigationUI()
        viewModel.loadMetadata(hash)
        loadMedia(hash, currentMediaType())
    }

    private fun rotateCurrentImage(direction: String) {
        if (isRotating || currentMediaType() == "video") return
        val hash = currentHash()
        if (hash.isEmpty()) return

        clearEnhanceState(restoreOriginal = false)
        isRotating = true
        rotateLeftButton.isEnabled = false
        rotateRightButton.isEnabled = false
        rotateLeftButton.alpha = 0.4f
        rotateRightButton.alpha = 0.4f
        loadingSpinner.visibility = View.VISIBLE

        lifecycleScope.launch {
            val result = RemoteMediaRepository(this@RemoteMediaDetailActivity).rotateImage(hash, direction)
            isRotating = false
            rotateLeftButton.isEnabled = true
            rotateRightButton.isEnabled = true
            rotateLeftButton.alpha = 1f
            rotateRightButton.alpha = 1f

            result.fold(
                onSuccess = { newOrientation ->
                    MediaSessionHolder.rotatedOrientations[hash] = newOrientation
                    Toast.makeText(this@RemoteMediaDetailActivity, "Rotated", Toast.LENGTH_SHORT).show()
                    viewModel.loadMetadata(hash)
                    loadMedia(hash, "image")
                },
                onFailure = { error ->
                    loadingSpinner.visibility = View.GONE
                    Toast.makeText(
                        this@RemoteMediaDetailActivity,
                        "Failed to rotate: ${error.message}",
                        Toast.LENGTH_SHORT
                    ).show()
                }
            )
        }
    }

    private fun loadMedia(hash: String, mediaType: String) {
        currentMediaJob?.cancel()
        currentMediaJob = null
        loadingSpinner.visibility = View.VISIBLE
        releasePlayer()

        val baseUrl = PreferenceHelper.getServerUrl(this).trimEnd('/')
        val isVideo = mediaType == "video"
        val endpoint = if (isVideo) "video" else "image"
        val rotatedOrient = if (!isVideo) MediaSessionHolder.rotatedOrientations[hash] else null
        val mediaUrl = if (rotatedOrient != null) {
            "$baseUrl/api/$endpoint/$hash?o=$rotatedOrient"
        } else {
            "$baseUrl/api/$endpoint/$hash"
        }

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
        currentMediaJob = lifecycleScope.launch {
            val token = withContext(Dispatchers.IO) {
                AuthHelper.getValidToken(this@RemoteMediaDetailActivity)
            } ?: run {
                loadingSpinner.visibility = View.GONE
                return@launch
            }

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
        currentMediaJob = lifecycleScope.launch {
            try {
                val token = withContext(Dispatchers.IO) {
                    AuthHelper.getValidToken(this@RemoteMediaDetailActivity)
                }
                if (token == null) {
                    loadingSpinner.visibility = View.GONE
                    return@launch
                }

                val tempFile = withContext(Dispatchers.IO) {
                    downloadImageToCache(mediaUrl, token, "rmd_")
                }

                if (tempFile != null && (isFinishing || isDestroyed)) {
                    tempFile.delete()
                } else {
                    currentTempFile?.delete()
                    currentTempFile = tempFile
                    if (tempFile != null) {
                        imagePreview.setImage(ImageSource.uri(tempFile.absolutePath))
                    }
                }
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e(TAG, "Error downloading image", e)
            } finally {
                loadingSpinner.visibility = View.GONE
            }
        }
    }

    private fun downloadImageToCache(mediaUrl: String, token: String, prefix: String): File? {
        val client = AuthenticatedHttpClient.getClient(this@RemoteMediaDetailActivity)
        val request = Request.Builder()
            .url(mediaUrl)
            .addHeader("Authorization", "Bearer $token")
            .build()
        return AuthHelper.executeWithTokenRefresh(this@RemoteMediaDetailActivity, client, request).use { response ->
            if (!response.isSuccessful) return null
            val file = File.createTempFile(prefix, ".jpg", cacheDir)
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
        val baseUrl = PreferenceHelper.getServerUrl(this).trimEnd('/')
        val isVideo = mediaType == "video"
        val endpoint = if (isVideo) "video" else "image"
        val rotatedOrient = if (!isVideo) MediaSessionHolder.rotatedOrientations[hash] else null
        val mediaUrl = if (rotatedOrient != null) {
            "$baseUrl/api/$endpoint/$hash?o=$rotatedOrient"
        } else {
            "$baseUrl/api/$endpoint/$hash"
        }

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
                    downloadImageToCache(mediaUrl, token, "cmp_${if (isLeft) "L" else "R"}_")
                }

                if (tempFile != null && !isFinishing && !isDestroyed) {
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

    // ── AI Enhance & Download ────────────────────────────────────────────────

    private fun showEnhanceModeDialog() {
        if (isEnhancing || currentMediaType() == "video") return
        val hash = currentHash()
        if (hash.isEmpty()) return

        val modes = listOf(
            "auto" to "✨ Auto Fix — Automatically detect and fix issues",
            "denoise" to "🧹 Denoise — Remove grain and compression noise",
            "restore" to "🎨 Restore Color — Fix faded colors, white balance & contrast",
            "lowlight" to "🌙 Low-Light HDR — Brighten dark photos cleanly",
            "sharpen" to "🔍 Sharpen — Crisp details and edge clarity",
            "face" to "👤 Face Enhance — Restore and sharpen facial details"
        )
        val labels = modes.map { it.second }.toTypedArray()

        AlertDialog.Builder(this)
            .setTitle("✨ AI Photo Enhancement")
            .setItems(labels) { _, which ->
                runEnhance(hash, modes[which].first)
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun runEnhance(hash: String, mode: String) {
        isEnhancing = true
        enhanceButton.isEnabled = false
        enhanceButton.alpha = 0.4f
        loadingSpinner.visibility = View.VISIBLE

        lifecycleScope.launch {
            val result = RemoteMediaRepository(this@RemoteMediaDetailActivity).enhanceImage(hash, mode)
            isEnhancing = false
            enhanceButton.isEnabled = true
            enhanceButton.alpha = 1f
            loadingSpinner.visibility = View.GONE

            result.fold(
                onSuccess = { enhanceResult ->
                    if (isFinishing || isDestroyed || currentHash() != hash) return@fold
                    val tempFile = withContext(Dispatchers.IO) {
                        File.createTempFile("enh_", ".jpg", cacheDir).apply {
                            writeBytes(enhanceResult.imageBytes)
                        }
                    }
                    enhancedTempFile?.delete()
                    enhancedTempFile = tempFile
                    enhancedBytes = enhanceResult.imageBytes
                    showingEnhanced = true

                    if (isCompareMode) {
                        isCompareMode = false
                        releaseComparePlayers()
                        syncCompareVisibility()
                    }

                    imagePreview.setImage(ImageSource.uri(tempFile.absolutePath))
                    val opsLabel = if (enhanceResult.operations.isNotEmpty()) {
                        "✨ Enhanced (${enhanceResult.operations.joinToString(", ")})"
                    } else {
                        "✨ Enhanced"
                    }
                    enhanceStatusText.text = opsLabel
                    enhanceToggleOriginalButton.text = "Show Original"
                    enhanceSaveButton.isEnabled = true
                    enhanceSaveButton.alpha = 1f
                    enhanceActionBar.visibility = View.VISIBLE
                },
                onFailure = { error ->
                    Toast.makeText(
                        this@RemoteMediaDetailActivity,
                        "Enhance failed: ${error.message}",
                        Toast.LENGTH_LONG
                    ).show()
                }
            )
        }
    }

    private fun toggleEnhancedPreview() {
        val enhancedFile = enhancedTempFile ?: return
        showingEnhanced = !showingEnhanced
        if (showingEnhanced) {
            imagePreview.setImage(ImageSource.uri(enhancedFile.absolutePath))
            enhanceToggleOriginalButton.text = "Show Original"
        } else {
            currentTempFile?.let { imagePreview.setImage(ImageSource.uri(it.absolutePath)) }
            enhanceToggleOriginalButton.text = "Show Enhanced"
        }
    }

    private fun saveEnhancedCopy() {
        val bytes = enhancedBytes ?: return
        val hash = currentHash()
        if (hash.isEmpty()) return

        enhanceSaveButton.isEnabled = false
        enhanceSaveButton.alpha = 0.5f
        loadingSpinner.visibility = View.VISIBLE

        lifecycleScope.launch {
            val result = RemoteMediaRepository(this@RemoteMediaDetailActivity).saveEnhancedImage(hash, bytes)
            loadingSpinner.visibility = View.GONE
            enhanceSaveButton.isEnabled = true
            enhanceSaveButton.alpha = 1f

            result.fold(
                onSuccess = {
                    MediaSessionHolder.reloadRequested = true
                    Toast.makeText(
                        this@RemoteMediaDetailActivity,
                        "Enhanced copy saved to library",
                        Toast.LENGTH_SHORT
                    ).show()
                    clearEnhanceState(restoreOriginal = false)
                },
                onFailure = { error ->
                    Toast.makeText(
                        this@RemoteMediaDetailActivity,
                        "Failed to save enhanced copy: ${error.message}",
                        Toast.LENGTH_LONG
                    ).show()
                }
            )
        }
    }

    private fun clearEnhanceState(restoreOriginal: Boolean) {
        enhancedTempFile?.delete()
        enhancedTempFile = null
        enhancedBytes = null
        showingEnhanced = false
        enhanceActionBar.visibility = View.GONE
        if (restoreOriginal) {
            currentTempFile?.let {
                if (it.exists()) imagePreview.setImage(ImageSource.uri(it.absolutePath))
            }
        }
    }

    private fun downloadCurrentMedia() {
        val hash = currentHash()
        if (hash.isEmpty()) return
        val isVideo = currentMediaType() == "video"
        val rawName = viewModel.metadata.value?.name?.takeIf { it.isNotBlank() }?.replace('/', '_')
        val isEnhancedActive = !isVideo && showingEnhanced && enhancedTempFile != null

        val fileName = when {
            isEnhancedActive -> {
                val stem = rawName?.substringBeforeLast('.') ?: hash.takeLast(12)
                "${stem}_enhanced.jpg"
            }
            rawName != null && rawName.contains('.') -> rawName
            rawName != null -> "$rawName.${if (isVideo) "mp4" else "jpg"}"
            else -> "${hash.takeLast(16)}.${if (isVideo) "mp4" else "jpg"}"
        }

        val ext = fileName.substringAfterLast('.', if (isVideo) "mp4" else "jpg").lowercase()
        val mimeType = when (ext) {
            "mp4" -> "video/mp4"
            "mov" -> "video/quicktime"
            "webm" -> "video/webm"
            "mkv" -> "video/x-matroska"
            "png" -> "image/png"
            "webp" -> "image/webp"
            else -> "image/jpeg"
        }

        downloadButton.isEnabled = false
        downloadButton.alpha = 0.4f
        Toast.makeText(this, "Downloading $fileName…", Toast.LENGTH_SHORT).show()

        val sourceFile: File? = when {
            isEnhancedActive -> enhancedTempFile
            !isVideo && currentTempFile?.exists() == true -> currentTempFile
            else -> null
        }

        val baseUrl = PreferenceHelper.getServerUrl(this).trimEnd('/')
        val endpoint = if (isVideo) "video" else "image"
        val oriented = if (!isVideo) MediaSessionHolder.rotatedOrientations[hash] else null
        val mediaUrl = if (oriented != null) {
            "$baseUrl/api/$endpoint/$hash?o=$oriented"
        } else {
            "$baseUrl/api/$endpoint/$hash"
        }

        lifecycleScope.launch {
            val saveResult = withContext(Dispatchers.IO) {
                runCatching {
                    writeToDownloads(fileName, mimeType) { out ->
                        if (sourceFile != null && sourceFile.exists()) {
                            FileInputStream(sourceFile).use { input -> input.copyTo(out) }
                        } else {
                            val token = AuthHelper.getValidToken(this@RemoteMediaDetailActivity)
                                ?: error("Not authenticated")
                            val client = AuthenticatedHttpClient.getClientWithTimeouts(
                                this@RemoteMediaDetailActivity, 30, 300
                            )
                            val request = Request.Builder()
                                .url(mediaUrl)
                                .addHeader("Authorization", "Bearer $token")
                                .build()
                            AuthHelper.executeWithTokenRefresh(
                                this@RemoteMediaDetailActivity, client, request
                            ).use { response ->
                                if (!response.isSuccessful) error("HTTP ${response.code}")
                                response.body?.byteStream()?.use { input -> input.copyTo(out) }
                                    ?: error("Empty download body")
                            }
                        }
                    }
                }
            }

            downloadButton.isEnabled = true
            downloadButton.alpha = 1f

            saveResult.fold(
                onSuccess = {
                    Toast.makeText(
                        this@RemoteMediaDetailActivity,
                        "Saved to Downloads: $fileName",
                        Toast.LENGTH_LONG
                    ).show()
                },
                onFailure = { error ->
                    Toast.makeText(
                        this@RemoteMediaDetailActivity,
                        "Download failed: ${error.message}",
                        Toast.LENGTH_LONG
                    ).show()
                }
            )
        }
    }

    private fun writeToDownloads(fileName: String, mimeType: String, writer: (OutputStream) -> Unit) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val values = ContentValues().apply {
                put(MediaStore.Downloads.DISPLAY_NAME, fileName)
                put(MediaStore.Downloads.MIME_TYPE, mimeType)
                put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
                put(MediaStore.Downloads.IS_PENDING, 1)
            }
            val uri = contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                ?: error("Could not create Downloads entry")
            try {
                contentResolver.openOutputStream(uri)?.use(writer)
                    ?: error("Could not open Downloads stream")
                values.clear()
                values.put(MediaStore.Downloads.IS_PENDING, 0)
                contentResolver.update(uri, values, null, null)
            } catch (e: Exception) {
                contentResolver.delete(uri, null, null)
                throw e
            }
        } else {
            @Suppress("DEPRECATION")
            val publicDownloads = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            val targetDir = try {
                if (publicDownloads.exists() || publicDownloads.mkdirs()) publicDownloads
                else getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: filesDir
            } catch (_: SecurityException) {
                getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: filesDir
            }
            val destFile = File(targetDir, fileName)
            FileOutputStream(destFile).use(writer)
            MediaScannerConnection.scanFile(this, arrayOf(destFile.absolutePath), arrayOf(mimeType), null)
        }
    }

    // ── Delete ────────────────────────────────────────────────────────────────

    private fun confirmAndDelete() {
        val hash = currentHash()
        if (hash.isEmpty()) return
        AlertDialog.Builder(this)
            .setTitle("Move to Trash")
            .setMessage("Move this item to Trash? You can restore it later from the Trash tab.")
            .setPositiveButton("Move to Trash") { _, _ ->
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

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putInt(STATE_POSITION, currentPosition)
    }

    override fun onDestroy() {
        super.onDestroy()
        currentMediaJob?.cancel()
        currentMediaJob = null
        releasePlayer()
        releaseComparePlayers()
        currentTempFile?.delete()
        currentTempFile = null
        enhancedTempFile?.delete()
        enhancedTempFile = null
        compareTempFileLeft?.delete()
        compareTempFileLeft = null
        compareTempFileRight?.delete()
        compareTempFileRight = null
        if (isFinishing && !isChangingConfigurations) {
            MediaSessionHolder.clearMediaSession()
        }
    }
}
