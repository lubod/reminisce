package org.openreminisce.app.fragments

import android.content.Intent
import android.os.Bundle
import android.text.Editable
import android.text.TextWatcher
import android.view.KeyEvent
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.SeekBar
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.fragment.app.Fragment
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.GridLayoutManager
import androidx.recyclerview.widget.RecyclerView
import androidx.swiperefreshlayout.widget.SwipeRefreshLayout
import com.google.android.material.chip.Chip
import com.google.android.material.chip.ChipGroup
import com.google.android.material.floatingactionbutton.FloatingActionButton
import com.google.android.material.textfield.TextInputEditText
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.openreminisce.app.R
import org.openreminisce.app.RemoteMediaAdapter
import org.openreminisce.app.RemoteMediaDetailActivity
import org.openreminisce.app.model.MediaFilter
import org.openreminisce.app.model.MediaTypeFilter
import org.openreminisce.app.model.SearchMode
import org.openreminisce.app.model.ThumbnailInfo
import org.openreminisce.app.repository.RemoteMediaRepository
import org.openreminisce.app.util.MediaSessionHolder

class RemoteMediaFragmentNative : Fragment(), FilterBottomSheetFragment.OnFiltersApplied {

    private lateinit var recyclerView: RecyclerView
    private lateinit var progressBar: ProgressBar
    private lateinit var swipeRefresh: SwipeRefreshLayout
    private lateinit var emptyStateText: TextView
    private lateinit var refreshFab: FloatingActionButton
    private lateinit var searchEditText: TextInputEditText
    private lateinit var filterButton: ImageButton
    private lateinit var filterChipGroup: ChipGroup
    private lateinit var chipAll: Chip
    private lateinit var chipImages: Chip
    private lateinit var chipVideos: Chip
    private lateinit var searchModeChipGroup: ChipGroup
    private lateinit var chipSemantic: Chip
    private lateinit var chipText: Chip
    private lateinit var chipHybrid: Chip
    private lateinit var similarityRow: LinearLayout
    private lateinit var similaritySeekBar: SeekBar
    private lateinit var similarityValueText: TextView
    private lateinit var groupByChip: Chip

    private lateinit var repository: RemoteMediaRepository

    private var allThumbnails = listOf<ThumbnailInfo>()
    private var currentPage = 1
    private var currentOffset = 0
    private var isLoading = false
    private var hasMore = true
    private var retryCount = 0
    private val MAX_RETRY = 3
    private val PAGE_LIMIT = 50
    private var loadGeneration = 0

    private var mediaAdapter: RemoteMediaAdapter? = null

    private var searchQuery = ""
    private var currentFilter = MediaFilter()

    /** ActivityResult launcher for RemoteMediaDetailActivity. */
    private val detailLauncher = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        var changed = false

        val deleted = MediaSessionHolder.deletedHashes
        if (deleted.isNotEmpty()) {
            allThumbnails = allThumbnails.filter { it.hash !in deleted }
            MediaSessionHolder.deletedHashes = mutableSetOf()
            changed = true
        }

        val updates = MediaSessionHolder.starredUpdates
        if (updates.isNotEmpty()) {
            allThumbnails = allThumbnails.map { thumb ->
                updates[thumb.hash]?.let { thumb.copy(starred = it) } ?: thumb
            }
            MediaSessionHolder.starredUpdates = mutableMapOf()
            changed = true
        }

        if (changed) updateUI()
    }

    private val isSearchMode get() = searchQuery.isNotEmpty()

    override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?): View? =
        inflater.inflate(R.layout.fragment_remote_media_native, container, false)

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        recyclerView = view.findViewById(R.id.remoteMediaRecyclerView)
        progressBar = view.findViewById(R.id.remoteMediaProgressBar)
        swipeRefresh = view.findViewById(R.id.remoteMediaSwipeRefresh)
        emptyStateText = view.findViewById(R.id.remoteMediaEmptyState)
        refreshFab = view.findViewById(R.id.remoteRefreshFab)
        searchEditText = view.findViewById(R.id.searchEditText)
        filterButton = view.findViewById(R.id.filterButton)
        filterChipGroup = view.findViewById(R.id.filterChipGroup)
        chipAll = view.findViewById(R.id.chipAll)
        chipImages = view.findViewById(R.id.chipImages)
        chipVideos = view.findViewById(R.id.chipVideos)
        searchModeChipGroup = view.findViewById(R.id.searchModeChipGroup)
        chipSemantic = view.findViewById(R.id.chipSemantic)
        chipText = view.findViewById(R.id.chipText)
        chipHybrid = view.findViewById(R.id.chipHybrid)
        similarityRow = view.findViewById(R.id.similarityRow)
        similaritySeekBar = view.findViewById(R.id.similaritySeekBar)
        similarityValueText = view.findViewById(R.id.similarityValueText)
        groupByChip = view.findViewById(R.id.groupByChip)

        repository = RemoteMediaRepository(requireContext())

        setupRecyclerView()
        setupSwipeRefresh()
        setupFab()
        setupMediaTypeChips()
        setupSearchModeChips()
        setupSearch()
        setupFilterButton()
        setupSimilaritySlider()
        setupGroupByChip()
        autoSelectSingleDevice()

        loadMedia()
    }

    // ── FilterBottomSheetFragment.OnFiltersApplied ────────────────────────

    override fun onFiltersApplied(filter: MediaFilter) {
        currentFilter = filter.copy(searchMode = currentSearchMode())
        updateFilterBadge()
        resetAndReload()
    }

    // ── Setup ─────────────────────────────────────────────────────────────

    private fun setupRecyclerView() {
        mediaAdapter = RemoteMediaAdapter(
            context = requireContext(),
            thumbnails = emptyList(),
            onMediaClick = { hash, position, isVideo ->
                val intent = Intent(requireContext(), RemoteMediaDetailActivity::class.java).apply {
                    putExtra(RemoteMediaDetailActivity.EXTRA_HASH, hash)
                    putExtra(RemoteMediaDetailActivity.EXTRA_POSITION, position)
                    putExtra("isVideo", isVideo)
                }
                detailLauncher.launch(intent)
            },
            onStarClick = { hash, mediaType ->
                toggleStarOptimistic(hash, mediaType)
            }
        )

        val gridLayoutManager = GridLayoutManager(requireContext(), 3).apply {
            spanSizeLookup = object : GridLayoutManager.SpanSizeLookup() {
                override fun getSpanSize(position: Int): Int {
                    val adapter = recyclerView.adapter as? RemoteMediaAdapter ?: return 1
                    return if (position < adapter.itemCount && adapter.getItemViewType(position) == 0) 3 else 1
                }
            }
        }
        recyclerView.layoutManager = gridLayoutManager
        recyclerView.setHasFixedSize(true)
        recyclerView.adapter = mediaAdapter

        recyclerView.addOnScrollListener(object : RecyclerView.OnScrollListener() {
            override fun onScrolled(recyclerView: RecyclerView, dx: Int, dy: Int) {
                if (!isLoading && hasMore && dy > 0) {
                    val visible = gridLayoutManager.childCount
                    val total = gridLayoutManager.itemCount
                    val firstVisible = gridLayoutManager.findFirstVisibleItemPosition()
                    if (visible + firstVisible >= total - 10) {
                        loadMoreMedia()
                    }
                }
            }
        })
    }

    private fun toggleStarOptimistic(hash: String, mediaType: String) {
        // Optimistic update in local list
        val prevList = allThumbnails
        val thumb = prevList.find { it.hash == hash } ?: return
        val optimisticStarred = !thumb.starred
        allThumbnails = prevList.map { if (it.hash == hash) it.copy(starred = optimisticStarred) else it }
        mediaAdapter?.updateData(allThumbnails)

        lifecycleScope.launch {
            val result = repository.toggleStar(hash, mediaType)
            result.fold(
                onSuccess = { starResponse ->
                    allThumbnails = allThumbnails.map {
                        if (it.hash == hash) it.copy(starred = starResponse.starred) else it
                    }
                    mediaAdapter?.updateData(allThumbnails)
                    // Persist for detail activity sync
                    MediaSessionHolder.starredUpdates[hash] = starResponse.starred
                },
                onFailure = {
                    // Revert
                    allThumbnails = allThumbnails.map {
                        if (it.hash == hash) it.copy(starred = thumb.starred) else it
                    }
                    mediaAdapter?.updateData(allThumbnails)
                    Toast.makeText(requireContext(), "Failed to toggle star", Toast.LENGTH_SHORT).show()
                }
            )
        }
    }

    private fun setupSwipeRefresh() {
        swipeRefresh.setOnRefreshListener { resetAndReload() }
    }

    private fun setupFab() {
        refreshFab.setOnClickListener { resetAndReload() }
    }

    private fun setupMediaTypeChips() {
        filterChipGroup.setOnCheckedStateChangeListener { _, checkedIds ->
            if (checkedIds.isEmpty()) return@setOnCheckedStateChangeListener
            val mediaType = when (checkedIds.first()) {
                R.id.chipImages -> MediaTypeFilter.IMAGE
                R.id.chipVideos -> MediaTypeFilter.VIDEO
                else -> MediaTypeFilter.ALL
            }
            if (currentFilter.mediaType != mediaType) {
                currentFilter = currentFilter.copy(mediaType = mediaType)
                resetAndReload()
            }
        }
    }

    private fun setupSearchModeChips() {
        searchModeChipGroup.setOnCheckedStateChangeListener { _, _ ->
            updateSimilarityRowVisibility()
            if (isSearchMode) resetAndReload()
        }
    }

    private fun setupSearch() {
        // Update chips visibility as user types, but do NOT trigger a search yet.
        searchEditText.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                val hasText = !s.isNullOrEmpty()
                searchModeChipGroup.visibility = if (hasText) View.VISIBLE else View.GONE
                updateSimilarityRowVisibility()

                // If the field was cleared, revert to browse mode immediately.
                if (!hasText && searchQuery.isNotEmpty()) {
                    searchQuery = ""
                    resetAndReload()
                }
            }
            override fun afterTextChanged(s: Editable?) {}
        })

        // Search fires only when the user presses the keyboard Search/Done action.
        searchEditText.setOnEditorActionListener { _, actionId, event ->
            val isSearch = actionId == EditorInfo.IME_ACTION_SEARCH
            val isEnter = event?.keyCode == KeyEvent.KEYCODE_ENTER && event.action == KeyEvent.ACTION_DOWN
            if (isSearch || isEnter) {
                val newQuery = searchEditText.text?.toString()?.trim() ?: ""
                if (newQuery != searchQuery) {
                    searchQuery = newQuery
                    resetAndReload()
                }
                // Hide keyboard
                val imm = requireContext().getSystemService(InputMethodManager::class.java)
                imm.hideSoftInputFromWindow(searchEditText.windowToken, 0)
                true
            } else false
        }

        searchEditText.setOnFocusChangeListener { _, hasFocus ->
            if (!hasFocus && searchQuery.isEmpty()) {
                searchModeChipGroup.visibility = View.GONE
                similarityRow.visibility = View.GONE
            }
        }
    }

    private fun setupFilterButton() {
        filterButton.setOnClickListener {
            FilterBottomSheetFragment.newInstance(
                currentFilter = currentFilter,
                showSearchMode = isSearchMode
            ).show(childFragmentManager, "filters")
        }
    }

    private fun setupSimilaritySlider() {
        // Init slider to current filter value (default 8%)
        val initProgress = (currentFilter.minSimilarity * 100).toInt()
        similaritySeekBar.progress = initProgress
        similarityValueText.text = "$initProgress%"

        similaritySeekBar.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(seekBar: SeekBar?, progress: Int, fromUser: Boolean) {
                if (fromUser) {
                    similarityValueText.text = "$progress%"
                }
            }

            override fun onStartTrackingTouch(seekBar: SeekBar?) {}

            override fun onStopTrackingTouch(seekBar: SeekBar?) {
                val newSimilarity = (seekBar?.progress ?: 8) / 100f
                if (currentFilter.minSimilarity != newSimilarity) {
                    currentFilter = currentFilter.copy(minSimilarity = newSimilarity)
                    if (isSearchMode) resetAndReload()
                }
            }
        })
    }

    private fun setupGroupByChip() {
        groupByChip.setOnCheckedChangeListener { _, isChecked ->
            mediaAdapter?.setGroupBy(isChecked)
        }
    }

    private fun updateSimilarityRowVisibility() {
        val show = isSearchMode && currentSearchMode() != SearchMode.TEXT
        similarityRow.visibility = if (show) View.VISIBLE else View.GONE
    }

    private fun autoSelectSingleDevice() {
        lifecycleScope.launch {
            repository.fetchDeviceIds().fold(
                onSuccess = { ids ->
                    if (ids.size == 1 && currentFilter.deviceId == null) {
                        currentFilter = currentFilter.copy(deviceId = ids[0])
                        resetAndReload()
                    }
                },
                onFailure = { /* silent */ }
            )
        }
    }

    // ── Data loading ──────────────────────────────────────────────────────

    private fun resetAndReload() {
        allThumbnails = emptyList()
        currentPage = 1
        currentOffset = 0
        hasMore = true
        retryCount = 0
        loadGeneration++
        isLoading = false
        loadMedia()
    }

    private fun loadMedia(isRetry: Boolean = false) {
        if (isLoading) return
        isLoading = true
        val capturedGeneration = loadGeneration

        if (!isRetry) progressBar.visibility = View.VISIBLE
        emptyStateText.visibility = View.GONE

        val filterWithMode = currentFilter.copy(searchMode = currentSearchMode())

        lifecycleScope.launch {
            val result = if (isSearchMode) {
                repository.searchMedia(
                    query = searchQuery,
                    offset = currentOffset,
                    limit = PAGE_LIMIT,
                    filter = filterWithMode
                )
            } else {
                repository.fetchRemoteThumbnails(
                    page = currentPage,
                    limit = PAGE_LIMIT,
                    filter = filterWithMode
                )
            }

            if (loadGeneration != capturedGeneration) {
                isLoading = false
                return@launch
            }

            result.fold(
                onSuccess = { response ->
                    retryCount = 0
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

                    if (retryCount < MAX_RETRY && !isRetry) {
                        retryCount++
                        Toast.makeText(requireContext(), "Retrying… ($retryCount/$MAX_RETRY)", Toast.LENGTH_SHORT).show()
                        lifecycleScope.launch {
                            delay(1000L * retryCount)
                            loadMedia(isRetry = true)
                        }
                    } else {
                        if (allThumbnails.isEmpty()) {
                            val msg = when {
                                error.message?.contains("not authenticated", true) == true ->
                                    "Please log in to view remote media"
                                error.message?.contains("not configured", true) == true ->
                                    "Server not configured"
                                error.message?.contains("HTTP 404") == true ->
                                    "API endpoint not found"
                                error.message?.contains("HTTP 500") == true ->
                                    "Server error — try again later"
                                else -> "Failed to load media: ${error.message}"
                            }
                            emptyStateText.text = msg
                            emptyStateText.visibility = View.VISIBLE
                            emptyStateText.setOnClickListener { retryCount = 0; resetAndReload() }
                        } else {
                            Toast.makeText(requireContext(), "Failed to load more: ${error.message}", Toast.LENGTH_LONG).show()
                        }
                    }
                }
            )
        }
    }

    private fun loadMoreMedia() {
        if (isSearchMode) {
            currentOffset += PAGE_LIMIT
        } else {
            currentPage++
        }
        loadMedia()
    }

    // ── UI helpers ────────────────────────────────────────────────────────

    private fun updateUI() {
        if (allThumbnails.isEmpty()) {
            val message = when {
                isSearchMode -> "No results for \"$searchQuery\""
                currentFilter.mediaType == MediaTypeFilter.IMAGE -> "No images found"
                currentFilter.mediaType == MediaTypeFilter.VIDEO -> "No videos found"
                else -> "No remote media found"
            }
            emptyStateText.text = message
            emptyStateText.visibility = View.VISIBLE
            recyclerView.visibility = View.GONE
        } else {
            emptyStateText.visibility = View.GONE
            recyclerView.visibility = View.VISIBLE
            mediaAdapter?.updateData(allThumbnails)
        }
    }

    private fun updateFilterBadge() {
        val hasActiveFilters = !currentFilter.copy(
            searchMode = MediaFilter().searchMode,
            minSimilarity = MediaFilter().minSimilarity
        ).isDefault()
        filterButton.alpha = if (hasActiveFilters) 1f else 0.6f
    }

    private fun currentSearchMode(): SearchMode = when (searchModeChipGroup.checkedChipId) {
        R.id.chipText -> SearchMode.TEXT
        R.id.chipHybrid -> SearchMode.HYBRID
        else -> SearchMode.SEMANTIC
    }

    companion object {
        fun newInstance() = RemoteMediaFragmentNative()
    }
}
