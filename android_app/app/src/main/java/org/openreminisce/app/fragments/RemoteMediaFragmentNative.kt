package org.openreminisce.app.fragments

import android.annotation.SuppressLint
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
import android.webkit.JavascriptInterface
import android.webkit.WebView
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
import com.google.android.material.chip.ChipGroup
import com.google.android.material.floatingactionbutton.FloatingActionButton
import com.google.android.material.textfield.TextInputEditText
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import org.openreminisce.app.R
import org.openreminisce.app.RemoteMediaAdapter
import org.openreminisce.app.RemoteMediaDetailActivity
import org.openreminisce.app.model.ImageInfo
import org.openreminisce.app.model.MapPoint
import org.openreminisce.app.model.MediaFilter
import org.openreminisce.app.model.MediaTypeFilter
import org.openreminisce.app.model.SearchMode
import org.openreminisce.app.model.ThumbnailInfo
import org.openreminisce.app.repository.RemoteMediaRepository
import org.openreminisce.app.util.MediaSessionHolder
import java.text.NumberFormat
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

class RemoteMediaFragmentNative : Fragment(), FilterBottomSheetFragment.OnFiltersApplied {

    private lateinit var recyclerView: RecyclerView
    private lateinit var progressBar: ProgressBar
    private lateinit var swipeRefresh: SwipeRefreshLayout
    private lateinit var emptyStateText: TextView
    private lateinit var refreshFab: FloatingActionButton
    private lateinit var searchEditText: TextInputEditText
    private lateinit var filterButton: ImageButton
    private lateinit var filterBadge: TextView
    private lateinit var searchOptionsContainer: LinearLayout
    private lateinit var searchModeChipGroup: ChipGroup
    private lateinit var similarityRow: LinearLayout
    private lateinit var similaritySeekBar: SeekBar
    private lateinit var similarityValueText: TextView
    private lateinit var totalCountText: TextView
    private lateinit var sortSummaryChip: TextView
    private lateinit var mapToggleChip: TextView
    private lateinit var resetFiltersButton: TextView
    private lateinit var mapWebView: WebView

    private lateinit var repository: RemoteMediaRepository

    private var allThumbnails = listOf<ThumbnailInfo>()
    private var mapPoints = listOf<MapPoint>()
    private var isMapView = false
    private var mapLoadedOnce = false
    private var totalMediaCount = 0
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
        if (MediaSessionHolder.reloadRequested) {
            MediaSessionHolder.reloadRequested = false
            MediaSessionHolder.deletedHashes = mutableSetOf()
            MediaSessionHolder.starredUpdates = mutableMapOf()
            MediaSessionHolder.rotatedOrientations = mutableMapOf()
            MediaSessionHolder.placeUpdates = mutableMapOf()
            resetAndReload()
            return@registerForActivityResult
        }

        var changed = false

        val deleted = MediaSessionHolder.deletedHashes
        if (deleted.isNotEmpty()) {
            val before = allThumbnails.size
            allThumbnails = allThumbnails.filter { it.hash !in deleted }
            val removed = before - allThumbnails.size
            if (removed > 0) {
                totalMediaCount = maxOf(0, totalMediaCount - removed)
            }
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

        val rotations = MediaSessionHolder.rotatedOrientations
        if (rotations.isNotEmpty()) {
            allThumbnails = allThumbnails.map { thumb ->
                rotations[thumb.hash]?.let { orient ->
                    thumb.copy(
                        orientation = orient,
                        thumbnailUrl = "/api/thumbnail/${thumb.hash}?o=$orient"
                    )
                } ?: thumb
            }
            MediaSessionHolder.rotatedOrientations = mutableMapOf()
            changed = true
        }

        val places = MediaSessionHolder.placeUpdates
        if (places.isNotEmpty()) {
            allThumbnails = allThumbnails.map { thumb ->
                if (thumb.hash in places) thumb.copy(place = places[thumb.hash]) else thumb
            }
            MediaSessionHolder.placeUpdates = mutableMapOf()
            changed = true
        }

        if (changed) {
            if (isMapView) {
                loadMapPoints()
            } else {
                updateUI()
                updateStatusAndFilterUi()
            }
        }
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
        filterBadge = view.findViewById(R.id.filterBadge)
        searchOptionsContainer = view.findViewById(R.id.searchOptionsContainer)
        searchModeChipGroup = view.findViewById(R.id.searchModeChipGroup)
        similarityRow = view.findViewById(R.id.similarityRow)
        similaritySeekBar = view.findViewById(R.id.similaritySeekBar)
        similarityValueText = view.findViewById(R.id.similarityValueText)
        totalCountText = view.findViewById(R.id.totalCountText)
        sortSummaryChip = view.findViewById(R.id.sortSummaryChip)
        mapToggleChip = view.findViewById(R.id.mapToggleChip)
        resetFiltersButton = view.findViewById(R.id.resetFiltersButton)
        mapWebView = view.findViewById(R.id.remoteMediaMapWebView)

        repository = RemoteMediaRepository(requireContext())

        setupRecyclerView()
        setupSwipeRefresh()
        setupFab()
        setupSearchModeChips()
        setupSearch()
        setupFilterControls()
        setupSimilaritySlider()
        setupMapView()
        updateStatusAndFilterUi()

        loadMedia()
    }

    // ── FilterBottomSheetFragment.OnFiltersApplied ────────────────────────

    override fun onFiltersApplied(filter: MediaFilter) {
        currentFilter = filter.copy(searchMode = currentSearchMode())
        mediaAdapter?.setGroupBy(currentFilter.groupByPlace)
        mediaAdapter?.setSortBy(currentFilter.sortBy)
        updateStatusAndFilterUi()
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
                    MediaSessionHolder.starredUpdates[hash] = starResponse.starred
                },
                onFailure = {
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

    private fun setupSearchModeChips() {
        searchModeChipGroup.setOnCheckedStateChangeListener { _, _ ->
            updateSimilarityRowVisibility()
            if (isSearchMode) resetAndReload()
        }
    }

    private fun setupSearch() {
        searchEditText.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                val hasText = !s.isNullOrEmpty()
                searchOptionsContainer.visibility = if (hasText) View.VISIBLE else View.GONE
                updateSimilarityRowVisibility()

                if (!hasText && searchQuery.isNotEmpty()) {
                    searchQuery = ""
                    updateStatusAndFilterUi()
                    resetAndReload()
                }
            }

            override fun afterTextChanged(s: Editable?) {}
        })

        searchEditText.setOnEditorActionListener { _, actionId, event ->
            val isSearch = actionId == EditorInfo.IME_ACTION_SEARCH
            val isEnter = event?.keyCode == KeyEvent.KEYCODE_ENTER && event.action == KeyEvent.ACTION_DOWN
            if (isSearch || isEnter) {
                val newQuery = searchEditText.text?.toString()?.trim() ?: ""
                if (newQuery != searchQuery) {
                    searchQuery = newQuery
                    updateStatusAndFilterUi()
                    resetAndReload()
                }
                val imm = requireContext().getSystemService(InputMethodManager::class.java)
                imm?.hideSoftInputFromWindow(searchEditText.windowToken, 0)
                true
            } else {
                false
            }
        }

        searchEditText.setOnFocusChangeListener { _, hasFocus ->
            if (!hasFocus && searchQuery.isEmpty() && searchEditText.text.isNullOrEmpty()) {
                searchOptionsContainer.visibility = View.GONE
                similarityRow.visibility = View.GONE
            }
        }
    }

    private fun setupFilterControls() {
        val openSheet = View.OnClickListener {
            FilterBottomSheetFragment.newInstance(
                currentFilter = currentFilter,
                showSearchMode = isSearchMode
            ).show(childFragmentManager, "filters")
        }
        filterButton.setOnClickListener(openSheet)
        sortSummaryChip.setOnClickListener(openSheet)

        resetFiltersButton.setOnClickListener {
            currentFilter = MediaFilter()
            mediaAdapter?.setGroupBy(false)
            mediaAdapter?.setSortBy("date")
            if (searchQuery.isNotEmpty() || !searchEditText.text.isNullOrEmpty()) {
                searchQuery = ""
                searchEditText.setText("")
                searchOptionsContainer.visibility = View.GONE
                similarityRow.visibility = View.GONE
            }
            updateStatusAndFilterUi()
            resetAndReload()
        }
    }

    private fun setupSimilaritySlider() {
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

    private fun updateSimilarityRowVisibility() {
        val show = (!searchEditText.text.isNullOrEmpty() || isSearchMode) && currentSearchMode() != SearchMode.TEXT
        similarityRow.visibility = if (show) View.VISIBLE else View.GONE
    }

    // ── Data loading ──────────────────────────────────────────────────────

    private fun resetAndReload() {
        if (isMapView) {
            loadMapPoints()
            return
        }
        allThumbnails = emptyList()
        totalMediaCount = 0
        currentPage = 1
        currentOffset = 0
        hasMore = true
        retryCount = 0
        loadGeneration++
        isLoading = false
        updateStatusAndFilterUi()
        loadMedia()
    }

    private fun loadMedia(isRetry: Boolean = false) {
        if (isLoading) return
        isLoading = true
        val capturedGeneration = loadGeneration

        if (!isRetry && allThumbnails.isEmpty()) {
            progressBar.visibility = View.VISIBLE
            updateStatusAndFilterUi()
        }
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
                    totalMediaCount = response.total
                    hasMore = allThumbnails.size < response.total

                    isLoading = false
                    progressBar.visibility = View.GONE
                    swipeRefresh.isRefreshing = false
                    updateUI()
                    updateStatusAndFilterUi()
                },
                onFailure = { error ->
                    isLoading = false
                    progressBar.visibility = View.GONE
                    swipeRefresh.isRefreshing = false
                    updateStatusAndFilterUi()

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

    private fun updateStatusAndFilterUi() {
        val activeCount = currentFilter.activeFilterCount()
        if (activeCount > 0) {
            filterBadge.text = activeCount.toString()
            filterBadge.visibility = View.VISIBLE
            filterButton.alpha = 1f
        } else {
            filterBadge.visibility = View.GONE
            filterButton.alpha = 0.7f
        }

        val sortLabel = when (currentFilter.sortBy) {
            "size" -> "Size"
            "quality" -> "Quality ★"
            else -> "Date"
        }
        val arrow = if (currentFilter.sortOrder == "asc") "↑" else "↓"
        val groupSuffix = if (currentFilter.groupByPlace) " • Place" else ""
        val typePrefix = when (currentFilter.mediaType) {
            MediaTypeFilter.IMAGE -> "Images • "
            MediaTypeFilter.VIDEO -> "Videos • "
            MediaTypeFilter.ALL -> ""
        }
        sortSummaryChip.text = "$typePrefix$sortLabel $arrow$groupSuffix"

        val numberFormat = NumberFormat.getIntegerInstance()
        val shown = numberFormat.format(allThumbnails.size)
        val total = numberFormat.format(totalMediaCount)
        totalCountText.text = when {
            isMapView && isLoading -> "Loading map..."
            isMapView -> "${numberFormat.format(mapPoints.size)} geolocated items"
            isLoading && allThumbnails.isEmpty() -> "Loading..."
            isSearchMode -> "Showing $shown / $total for \"$searchQuery\""
            else -> "Showing $shown / $total items"
        }

        val hasNonDefaultState = activeCount > 0 || isSearchMode
        resetFiltersButton.visibility = if (hasNonDefaultState) View.VISIBLE else View.GONE
    }

    // ── Map view ──────────────────────────────────────────────────────────

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupMapView() {
        mapWebView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
        }
        mapWebView.setBackgroundColor(0xFF0F172A.toInt())
        mapWebView.addJavascriptInterface(MapJsBridge(), "AndroidMapBridge")

        mapToggleChip.setOnClickListener { toggleMapView() }
    }

    private fun toggleMapView() {
        isMapView = !isMapView
        mapToggleChip.text = if (isMapView) "⊞ Grid" else "🗺 Map"

        if (isMapView) {
            swipeRefresh.visibility = View.GONE
            refreshFab.visibility = View.GONE
            emptyStateText.visibility = View.GONE
            mapWebView.visibility = View.VISIBLE
            loadMapPoints()
        } else {
            mapWebView.visibility = View.GONE
            swipeRefresh.visibility = View.VISIBLE
            refreshFab.visibility = View.VISIBLE
            updateUI()
            updateStatusAndFilterUi()
        }
    }

    private fun loadMapPoints() {
        isLoading = true
        progressBar.visibility = View.VISIBLE
        emptyStateText.visibility = View.GONE
        updateStatusAndFilterUi()

        lifecycleScope.launch {
            val result = repository.fetchMapPoints(currentFilter, page = 1, limit = 2000)
            if (!isAdded || !isMapView) {
                isLoading = false
                progressBar.visibility = View.GONE
                return@launch
            }
            isLoading = false
            progressBar.visibility = View.GONE

            result.fold(
                onSuccess = { response ->
                    mapPoints = response.points
                    updateStatusAndFilterUi()
                    if (mapPoints.isEmpty()) {
                        mapWebView.visibility = View.GONE
                        emptyStateText.text = "No geolocated photos found"
                        emptyStateText.visibility = View.VISIBLE
                    } else {
                        mapWebView.visibility = View.VISIBLE
                        emptyStateText.visibility = View.GONE
                        renderMapPoints(mapPoints)
                    }
                },
                onFailure = { error ->
                    updateStatusAndFilterUi()
                    if (mapPoints.isEmpty()) {
                        mapWebView.visibility = View.GONE
                        emptyStateText.text = "Failed to load map: ${error.message}"
                        emptyStateText.visibility = View.VISIBLE
                        emptyStateText.setOnClickListener { loadMapPoints() }
                    } else {
                        Toast.makeText(
                            requireContext(),
                            "Failed to update map: ${error.message}",
                            Toast.LENGTH_SHORT
                        ).show()
                    }
                }
            )
        }
    }

    private fun renderMapPoints(points: List<MapPoint>) {
        val jsonArray = JSONArray()
        for (p in points) {
            jsonArray.put(JSONObject().apply {
                put("hash", p.hash)
                put("lat", p.lat)
                put("lon", p.lon)
                put("place", p.place ?: "")
                put("created_at", p.created_at)
                put("starred", p.starred)
            })
        }
        val safeJson = jsonArray.toString().replace("<", "\\u003c")
        if (!mapLoadedOnce) {
            mapLoadedOnce = true
            mapWebView.loadDataWithBaseURL("https://openstreetmap.org/", buildMapHtml(safeJson), "text/html", "UTF-8", null)
        } else {
            mapWebView.evaluateJavascript("if (window.renderReminiscePoints) window.renderReminiscePoints($safeJson);", null)
        }
    }

    private fun buildMapHtml(initialJson: String): String = """
        <!DOCTYPE html>
        <html>
        <head>
          <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no" />
          <link rel="stylesheet" href="https://unpkg.com/leaflet@1.9.4/dist/leaflet.css" />
          <link rel="stylesheet" href="https://unpkg.com/leaflet.markercluster@1.5.3/dist/MarkerCluster.css" />
          <link rel="stylesheet" href="https://unpkg.com/leaflet.markercluster@1.5.3/dist/MarkerCluster.Default.css" />
          <style>
            html, body, #map { margin: 0; padding: 0; width: 100%; height: 100%; background: #0f172a; font-family: sans-serif; }
            .popup-card { min-width: 150px; }
            .popup-title { font-weight: 700; font-size: 13px; margin-bottom: 3px; color: #0f172a; }
            .popup-date { font-size: 11px; color: #475569; margin-bottom: 8px; }
            .popup-btn { background: #2563eb; color: #fff; border: none; border-radius: 6px; padding: 6px 10px; font-size: 12px; font-weight: 600; width: 100%; }
          </style>
        </head>
        <body>
          <div id="map"></div>
          <script id="initial-points" type="application/json">$initialJson</script>
          <script src="https://unpkg.com/leaflet@1.9.4/dist/leaflet.js"></script>
          <script src="https://unpkg.com/leaflet.markercluster@1.5.3/dist/leaflet.markercluster.js"></script>
          <script>
            function esc(str) {
              return String(str || '').replace(/[&<>"']/g, function(c) {
                return {'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c];
              });
            }
            var map = L.map('map').setView([20, 0], 2);
            L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
              attribution: '&copy; OpenStreetMap',
              maxZoom: 19
            }).addTo(map);
            var clusterGroup = typeof L.markerClusterGroup === 'function' ? L.markerClusterGroup() : L.layerGroup();
            map.addLayer(clusterGroup);

            window.renderReminiscePoints = function(points) {
              clusterGroup.clearLayers();
              if (!Array.isArray(points) || points.length === 0) return;
              var bounds = [];
              points.forEach(function(pt) {
                var lat = Number(pt.lat), lon = Number(pt.lon);
                if (!isFinite(lat) || !isFinite(lon)) return;
                bounds.push([lat, lon]);
                var marker = L.marker([lat, lon]);
                var label = esc(pt.place || (lat.toFixed(4) + ', ' + lon.toFixed(4)));
                var date = esc((pt.created_at || '').slice(0, 10));
                var star = pt.starred ? '★ ' : '';
                var html = '<div class="popup-card">' +
                  '<div class="popup-title">' + star + label + '</div>' +
                  '<div class="popup-date">' + date + '</div>' +
                  '<button class="popup-btn" data-hash="' + esc(pt.hash) + '" onclick="AndroidMapBridge.openMedia(this.getAttribute(\'data-hash\'))">Open Photo</button>' +
                  '</div>';
                marker.bindPopup(html);
                clusterGroup.addLayer(marker);
              });
              if (bounds.length > 0) {
                map.fitBounds(bounds, { padding: [30, 30], maxZoom: 14 });
              }
            };

            try {
              var initial = JSON.parse(document.getElementById('initial-points').textContent || '[]');
              window.renderReminiscePoints(initial);
            } catch (e) {}
          </script>
        </body>
        </html>
    """.trimIndent()

    private inner class MapJsBridge {
        @JavascriptInterface
        fun openMedia(hash: String) {
            if (!isAdded || hash.isEmpty()) return
            requireActivity().runOnUiThread {
                val points = mapPoints
                val index = points.indexOfFirst { it.hash == hash }.takeIf { it >= 0 } ?: 0
                val utcParser = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss", Locale.US).apply {
                    timeZone = TimeZone.getTimeZone("UTC")
                }
                MediaSessionHolder.hashes = points.map { it.hash }
                MediaSessionHolder.imageInfos = points.map { p ->
                    val parsedDate = try {
                        utcParser.parse(p.created_at.substring(0, minOf(19, p.created_at.length))) ?: Date()
                    } catch (_: Exception) {
                        Date()
                    }
                    ImageInfo(
                        id = p.hash,
                        date = parsedDate,
                        place = p.place,
                        mediaType = "image",
                        hash = p.hash
                    )
                }
                val intent = Intent(requireContext(), RemoteMediaDetailActivity::class.java).apply {
                    putExtra(RemoteMediaDetailActivity.EXTRA_HASH, hash)
                    putExtra(RemoteMediaDetailActivity.EXTRA_POSITION, index)
                    putExtra("isVideo", false)
                }
                detailLauncher.launch(intent)
            }
        }
    }

    override fun onDestroyView() {
        if (::mapWebView.isInitialized) {
            mapWebView.destroy()
        }
        mapLoadedOnce = false
        super.onDestroyView()
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
