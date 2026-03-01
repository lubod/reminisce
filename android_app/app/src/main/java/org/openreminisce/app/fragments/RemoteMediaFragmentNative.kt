package org.openreminisce.app.fragments

import android.os.Bundle
import android.text.Editable
import android.text.TextWatcher
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ProgressBar
import android.widget.TextView
import android.widget.Toast
import androidx.fragment.app.Fragment
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.GridLayoutManager
import androidx.recyclerview.widget.RecyclerView
import androidx.swiperefreshlayout.widget.SwipeRefreshLayout
import org.openreminisce.app.R
import org.openreminisce.app.RemoteMediaAdapter
import org.openreminisce.app.model.ThumbnailInfo
import org.openreminisce.app.repository.RemoteMediaRepository
import com.google.android.material.chip.Chip
import com.google.android.material.chip.ChipGroup
import com.google.android.material.floatingactionbutton.FloatingActionButton
import com.google.android.material.textfield.TextInputEditText
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

class RemoteMediaFragmentNative : Fragment() {

    private lateinit var recyclerView: RecyclerView
    private lateinit var progressBar: ProgressBar
    private lateinit var swipeRefresh: SwipeRefreshLayout
    private lateinit var emptyStateText: TextView
    private lateinit var refreshFab: FloatingActionButton
    private lateinit var repository: RemoteMediaRepository
    private lateinit var searchEditText: TextInputEditText
    private lateinit var filterChipGroup: ChipGroup
    private lateinit var chipAll: Chip
    private lateinit var chipImages: Chip
    private lateinit var chipVideos: Chip

    private var allThumbnails = listOf<ThumbnailInfo>()
    private var filteredThumbnails = listOf<ThumbnailInfo>()
    private var currentPage = 1
    private var isLoading = false
    private var hasMore = true
    private var currentFilter = FilterType.ALL
    private var searchQuery = ""
    private var searchJob: Job? = null
    private var retryCount = 0
    private val MAX_RETRY = 3

    enum class FilterType {
        ALL, IMAGES, VIDEOS
    }

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
        searchEditText = view.findViewById(R.id.searchEditText)
        filterChipGroup = view.findViewById(R.id.filterChipGroup)
        chipAll = view.findViewById(R.id.chipAll)
        chipImages = view.findViewById(R.id.chipImages)
        chipVideos = view.findViewById(R.id.chipVideos)

        repository = RemoteMediaRepository(requireContext())

        setupRecyclerView()
        setupSwipeRefresh()
        setupFab()
        setupFilters()
        setupSearch()

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

    private fun setupFilters() {
        filterChipGroup.setOnCheckedStateChangeListener { _, checkedIds ->
            if (checkedIds.isEmpty()) return@setOnCheckedStateChangeListener

            currentFilter = when (checkedIds.first()) {
                R.id.chipAll -> FilterType.ALL
                R.id.chipImages -> FilterType.IMAGES
                R.id.chipVideos -> FilterType.VIDEOS
                else -> FilterType.ALL
            }

            applyFilters()
        }
    }

    private fun setupSearch() {
        searchEditText.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}

            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                // Cancel previous search job
                searchJob?.cancel()

                // Debounce search: wait 300ms before executing
                searchJob = lifecycleScope.launch {
                    delay(300)
                    searchQuery = s?.toString() ?: ""
                    applyFilters()
                }
            }

            override fun afterTextChanged(s: Editable?) {}
        })
    }

    private fun applyFilters() {
        // Start with all thumbnails
        var filtered = allThumbnails

        // Apply media type filter
        filtered = when (currentFilter) {
            FilterType.IMAGES -> filtered.filter { it.mediaType == "image" }
            FilterType.VIDEOS -> filtered.filter { it.mediaType == "video" }
            FilterType.ALL -> filtered
        }

        // Apply search query
        if (searchQuery.isNotEmpty()) {
            val query = searchQuery.lowercase()
            filtered = filtered.filter { thumbnail ->
                // Search in date
                val dateMatches = thumbnail.created_at.lowercase().contains(query)
                // Search in location
                val locationMatches = thumbnail.place?.lowercase()?.contains(query) == true

                dateMatches || locationMatches
            }
        }

        filteredThumbnails = filtered
        updateUI()
    }

    private fun loadRemoteMedia(isRetry: Boolean = false) {
        if (isLoading) return

        isLoading = true
        if (!isRetry) {
            progressBar.visibility = View.VISIBLE
        }
        emptyStateText.visibility = View.GONE

        lifecycleScope.launch {
            repository.fetchRemoteThumbnails(page = currentPage).fold(
                onSuccess = { response ->
                    retryCount = 0 // Reset retry count on success
                    allThumbnails = allThumbnails + response.thumbnails
                    hasMore = allThumbnails.size < response.total

                    applyFilters()
                    isLoading = false
                    progressBar.visibility = View.GONE
                    swipeRefresh.isRefreshing = false
                },
                onFailure = { error ->
                    isLoading = false
                    progressBar.visibility = View.GONE
                    swipeRefresh.isRefreshing = false

                    // Retry logic
                    if (retryCount < MAX_RETRY && !isRetry) {
                        retryCount++
                        Toast.makeText(
                            requireContext(),
                            "Retrying... (${retryCount}/$MAX_RETRY)",
                            Toast.LENGTH_SHORT
                        ).show()

                        lifecycleScope.launch {
                            delay(1000L * retryCount) // Exponential backoff
                            loadRemoteMedia(isRetry = true)
                        }
                    } else {
                        // Show error after all retries exhausted
                        if (allThumbnails.isEmpty()) {
                            val errorMessage = when {
                                error.message?.contains("not authenticated") == true ->
                                    "Please log in to view remote media"
                                error.message?.contains("not configured") == true ->
                                    "Server not configured"
                                error.message?.contains("HTTP 404") == true ->
                                    "API endpoint not found. Please check server configuration."
                                error.message?.contains("HTTP 500") == true ->
                                    "Server error. Please try again later."
                                else -> "Failed to load remote media: ${error.message}"
                            }

                            emptyStateText.text = errorMessage
                            emptyStateText.visibility = View.VISIBLE
                            emptyStateText.setOnClickListener {
                                retryCount = 0
                                refreshMedia()
                            }
                        } else {
                            Toast.makeText(
                                requireContext(),
                                "Failed to load more: ${error.message}",
                                Toast.LENGTH_LONG
                            ).show()
                        }
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
        filteredThumbnails = emptyList()
        currentPage = 1
        hasMore = true
        retryCount = 0
        loadRemoteMedia()
    }

    private fun updateUI() {
        if (filteredThumbnails.isEmpty()) {
            val message = when {
                allThumbnails.isEmpty() -> "No remote media found"
                searchQuery.isNotEmpty() -> "No results for \"$searchQuery\""
                currentFilter == FilterType.IMAGES -> "No images found"
                currentFilter == FilterType.VIDEOS -> "No videos found"
                else -> "No media found"
            }

            emptyStateText.text = message
            emptyStateText.visibility = View.VISIBLE
            recyclerView.visibility = View.GONE
        } else {
            emptyStateText.visibility = View.GONE
            recyclerView.visibility = View.VISIBLE

            val adapter = RemoteMediaAdapter(requireContext(), filteredThumbnails)
            recyclerView.adapter = adapter
        }
    }

    companion object {
        fun newInstance() = RemoteMediaFragmentNative()
    }
}