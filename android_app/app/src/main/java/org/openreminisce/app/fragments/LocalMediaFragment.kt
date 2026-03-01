package org.openreminisce.app.fragments

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ProgressBar
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.core.view.MenuProvider
import androidx.fragment.app.Fragment
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.GridLayoutManager
import androidx.recyclerview.widget.RecyclerView
import androidx.swiperefreshlayout.widget.SwipeRefreshLayout
import org.openreminisce.app.ImagePreviewActivity
import org.openreminisce.app.LocalMediaAdapter
import org.openreminisce.app.R
import org.openreminisce.app.repository.LocalMediaRepository
import org.openreminisce.app.util.SnackbarHelper
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class LocalMediaFragment : Fragment() {

    companion object {
        private const val TAG = "LocalMediaFragment"
    }

    private lateinit var localMediaRecyclerView: RecyclerView
    private lateinit var localMediaAdapter: LocalMediaAdapter
    private lateinit var localMediaProgressBar: ProgressBar
    private lateinit var localMediaEmptyState: TextView
    private lateinit var swipeRefreshLayout: SwipeRefreshLayout
    private lateinit var localMediaRepository: LocalMediaRepository
    private var localMediaLoadJob: Job? = null

    // Permission request launcher for local gallery
    private val mediaPermissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { permissions ->
        val allGranted = permissions.values.all { it }
        if (allGranted) {
            Log.d(TAG, "Media permissions granted, loading local gallery")
            loadLocalMediaGallery()
        } else {
            Log.w(TAG, "Media permissions denied")
            // Handle permission denial if needed
        }
    }

    override fun onCreateView(
        inflater: LayoutInflater,
        container: ViewGroup?,
        savedInstanceState: Bundle?
    ): View? {
        return inflater.inflate(R.layout.fragment_local_media, container, false)
    }

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        localMediaRecyclerView = view.findViewById(R.id.localMediaRecyclerView)
        localMediaProgressBar = view.findViewById(R.id.localMediaProgressBar)
        localMediaEmptyState = view.findViewById(R.id.localMediaEmptyState)
        swipeRefreshLayout = view.findViewById<androidx.swiperefreshlayout.widget.SwipeRefreshLayout>(R.id.localMediaSwipeRefresh)
        val refreshFab: com.google.android.material.floatingactionbutton.FloatingActionButton = view.findViewById(R.id.refreshFab)
        localMediaRepository = LocalMediaRepository()

        setupLocalMediaRecyclerView()
        setupSwipeRefresh()
        setupMenu()

        refreshFab.setOnClickListener {
            loadLocalMediaGallery(isRefresh = true)
        }

        if (hasMediaPermissions()) {
            loadLocalMediaGallery()
        } else {
            requestMediaPermissions()
        }
    }

    private fun setupMenu() {
        requireActivity().addMenuProvider(object : MenuProvider {
            override fun onCreateMenu(menu: android.view.Menu, menuInflater: android.view.MenuInflater) {
                // Menu is already inflated in MainActivity
            }

            override fun onMenuItemSelected(menuItem: android.view.MenuItem): Boolean {
                return when (menuItem.itemId) {
                    R.id.action_refresh -> {
                        loadLocalMediaGallery(isRefresh = true)
                        true
                    }
                    else -> false
                }
            }
        }, viewLifecycleOwner, Lifecycle.State.RESUMED)
    }

    private fun setupLocalMediaRecyclerView() {
        localMediaAdapter = LocalMediaAdapter { imageInfo ->
            // Handle media click - open preview activity
            val intent = Intent(requireContext(), ImagePreviewActivity::class.java)
            intent.putExtra("imageHash", imageInfo.id)
            intent.putExtra("isLocalMedia", true)
            startActivity(intent)
        }

        val gridLayoutManager = GridLayoutManager(requireContext(), 3)
        gridLayoutManager.spanSizeLookup = object : GridLayoutManager.SpanSizeLookup() {
            override fun getSpanSize(position: Int): Int {
                return when (localMediaAdapter.getItemViewType(position)) {
                    LocalMediaAdapter.TYPE_HEADER -> 3 // Header spans 3 columns (full width)
                    else -> 1 // Media item spans 1 column
                }
            }
        }

        localMediaRecyclerView.apply {
            layoutManager = gridLayoutManager
            adapter = localMediaAdapter
        }
    }

    private fun setupSwipeRefresh() {
        swipeRefreshLayout.setOnRefreshListener {
            loadLocalMediaGallery(isRefresh = true)
        }
    }

    private fun loadLocalMediaGallery(isRefresh: Boolean = false) {
        localMediaLoadJob?.cancel()
        localMediaLoadJob = viewLifecycleOwner.lifecycleScope.launch {
            try {
                if (!isRefresh) {
                    localMediaProgressBar.visibility = View.VISIBLE
                }
                localMediaEmptyState.visibility = View.GONE

                val mediaItems = withContext(Dispatchers.IO) {
                    localMediaRepository.loadLocalMediaWithBackupStatus(requireContext())
                }

                localMediaAdapter.updateItems(mediaItems)
                localMediaProgressBar.visibility = View.GONE
                swipeRefreshLayout.isRefreshing = false

                // Show empty state if no media found
                if (mediaItems.isEmpty()) {
                    localMediaEmptyState.visibility = View.VISIBLE
                    localMediaRecyclerView.visibility = View.GONE
                } else {
                    localMediaEmptyState.visibility = View.GONE
                    localMediaRecyclerView.visibility = View.VISIBLE
                }

                Log.d(TAG, "Loaded ${mediaItems.size} local media items")
            } catch (e: Exception) {
                Log.e(TAG, "Error loading local media", e)
                localMediaProgressBar.visibility = View.GONE
                swipeRefreshLayout.isRefreshing = false
            }
        }
    }

    private fun hasMediaPermissions(): Boolean {
        val context = requireContext()
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            // Android 13+ requires READ_MEDIA_IMAGES and READ_MEDIA_VIDEO
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_IMAGES) == PackageManager.PERMISSION_GRANTED &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_VIDEO) == PackageManager.PERMISSION_GRANTED
        } else {
            // Android 12 and below use READ_EXTERNAL_STORAGE
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED
        }
    }

    private fun requestMediaPermissions() {
        val permissions = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            arrayOf(
                Manifest.permission.READ_MEDIA_IMAGES,
                Manifest.permission.READ_MEDIA_VIDEO
            )
        } else {
            arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        }

        Log.d(TAG, "Requesting media permissions: ${permissions.joinToString()}")
        mediaPermissionLauncher.launch(permissions)
    }

    override fun onDestroyView() {
        super.onDestroyView()
        localMediaLoadJob?.cancel()
    }
}