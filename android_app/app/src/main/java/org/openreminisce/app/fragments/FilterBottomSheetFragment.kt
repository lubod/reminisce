package org.openreminisce.app.fragments

import android.os.Bundle
import android.text.Editable
import android.text.TextWatcher
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ArrayAdapter
import android.widget.Filter
import android.widget.TextView
import androidx.lifecycle.lifecycleScope
import com.google.android.material.bottomsheet.BottomSheetDialogFragment
import com.google.android.material.button.MaterialButton
import com.google.android.material.chip.ChipGroup
import com.google.android.material.datepicker.MaterialDatePicker
import com.google.android.material.slider.Slider
import com.google.android.material.switchmaterial.SwitchMaterial
import com.google.android.material.textfield.MaterialAutoCompleteTextView
import com.google.android.material.textfield.TextInputEditText
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.openreminisce.app.R
import org.openreminisce.app.model.Label
import org.openreminisce.app.model.LocationResult
import org.openreminisce.app.model.MediaFilter
import org.openreminisce.app.model.MediaTypeFilter
import org.openreminisce.app.repository.LabelRepository
import org.openreminisce.app.repository.RemoteMediaRepository
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class FilterBottomSheetFragment : BottomSheetDialogFragment() {

    interface OnFiltersApplied {
        fun onFiltersApplied(filter: MediaFilter)
    }

    companion object {
        private const val ARG_SHOW_SEARCH_MODE = "show_search_mode"
        private const val ANY_DEVICE_LABEL = "Any device"
        private const val ALL_LABELS_LABEL = "All labels"

        fun newInstance(
            currentFilter: MediaFilter = MediaFilter(),
            showSearchMode: Boolean = false
        ) = FilterBottomSheetFragment().apply {
            arguments = Bundle().apply {
                putBoolean(ARG_SHOW_SEARCH_MODE, showSearchMode)
                putString("media_type", currentFilter.mediaType.name)
                putLong("start_date", currentFilter.startDate ?: -1L)
                putLong("end_date", currentFilter.endDate ?: -1L)
                putBoolean("starred_only", currentFilter.starredOnly)
                putString("device_id", currentFilter.deviceId ?: "")
                putInt("label_id", currentFilter.labelId ?: -1)
                putDouble("lat", currentFilter.locationLat ?: Double.NaN)
                putDouble("lon", currentFilter.locationLon ?: Double.NaN)
                putFloat("radius_km", currentFilter.locationRadiusKm)
                putFloat("min_similarity", currentFilter.minSimilarity)
                putString("sort_by", currentFilter.sortBy)
                putString("sort_order", currentFilter.sortOrder)
                putBoolean("group_by_place", currentFilter.groupByPlace)
            }
        }
    }

    private val displayDateFormat = SimpleDateFormat("MMM d, yyyy", Locale.getDefault())

    private var startDateMs: Long? = null
    private var endDateMs: Long? = null
    private var selectedLabelId: Int? = null
    private var selectedLocationResult: LocationResult? = null
    private var sortOrderValue: String = "desc"
    private var locationSearchJob: Job? = null
    private var availableLabels: List<Label> = emptyList()
    private val locationSuggestions = mutableListOf<LocationResult>()
    private var locationAdapter: ArrayAdapter<LocationResult>? = null

    override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?): View? =
        inflater.inflate(R.layout.fragment_filter_bottom_sheet, container, false)

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        val sortChipGroup = view.findViewById<ChipGroup>(R.id.sortChipGroup)
        val sortOrderButton = view.findViewById<MaterialButton>(R.id.sortOrderButton)
        val groupByChipGroup = view.findViewById<ChipGroup>(R.id.groupByChipGroup)
        val startDateField = view.findViewById<TextInputEditText>(R.id.startDateField)
        val endDateField = view.findViewById<TextInputEditText>(R.id.endDateField)
        val clearDatesButton = view.findViewById<MaterialButton>(R.id.clearDatesButton)
        val starredSwitch = view.findViewById<SwitchMaterial>(R.id.starredSwitch)
        val mediaTypeChipGroup = view.findViewById<ChipGroup>(R.id.mediaTypeChipGroup)
        val deviceDropdown = view.findViewById<MaterialAutoCompleteTextView>(R.id.deviceDropdown)
        val labelDropdown = view.findViewById<MaterialAutoCompleteTextView>(R.id.labelDropdown)
        val locationField = view.findViewById<MaterialAutoCompleteTextView>(R.id.locationSearchField)
        val clearLocationButton = view.findViewById<MaterialButton>(R.id.clearLocationButton)
        val radiusSection = view.findViewById<View>(R.id.radiusSection)
        val radiusSlider = view.findViewById<Slider>(R.id.locationRadiusSlider)
        val applyButton = view.findViewById<View>(R.id.applyButton)
        val clearButton = view.findViewById<View>(R.id.clearButton)

        val args = arguments ?: Bundle()

        // Sort & Order
        when (args.getString("sort_by", "date")) {
            "size" -> sortChipGroup.check(R.id.sortSizeChip)
            "quality" -> sortChipGroup.check(R.id.sortQualityChip)
            else -> sortChipGroup.check(R.id.sortDateChip)
        }
        sortOrderValue = if (args.getString("sort_order", "desc") == "asc") "asc" else "desc"
        updateSortOrderButtonText(sortOrderButton)
        sortOrderButton.setOnClickListener {
            sortOrderValue = if (sortOrderValue == "asc") "desc" else "asc"
            updateSortOrderButtonText(sortOrderButton)
        }

        // Group by
        if (args.getBoolean("group_by_place", false)) {
            groupByChipGroup.check(R.id.groupByPlaceChip)
        } else {
            groupByChipGroup.check(R.id.groupByDateChip)
        }

        // Dates
        val savedStartDate = args.getLong("start_date", -1L).takeIf { it != -1L }
        val savedEndDate = args.getLong("end_date", -1L).takeIf { it != -1L }
        startDateMs = savedStartDate
        endDateMs = savedEndDate
        savedStartDate?.let { startDateField.setText(displayDateFormat.format(Date(it))) }
        savedEndDate?.let { endDateField.setText(displayDateFormat.format(Date(it))) }
        clearDatesButton.visibility = if (startDateMs != null || endDateMs != null) View.VISIBLE else View.GONE

        // Starred
        starredSwitch.isChecked = args.getBoolean("starred_only", false)

        // Media type
        val savedMediaType = runCatching {
            MediaTypeFilter.valueOf(args.getString("media_type", "ALL") ?: "ALL")
        }.getOrDefault(MediaTypeFilter.ALL)
        when (savedMediaType) {
            MediaTypeFilter.IMAGE -> mediaTypeChipGroup.check(R.id.filterChipImages)
            MediaTypeFilter.VIDEO -> mediaTypeChipGroup.check(R.id.filterChipVideos)
            else -> mediaTypeChipGroup.check(R.id.filterChipAll)
        }

        // Radius slider (hidden until a location is picked)
        val savedRadius = args.getFloat("radius_km", 10f)
        radiusSlider.value = savedRadius.coerceIn(1f, 500f)
        val savedLat = args.getDouble("lat", Double.NaN)
        val savedLon = args.getDouble("lon", Double.NaN)
        if (!savedLat.isNaN() && !savedLon.isNaN()) {
            selectedLocationResult = LocationResult(
                name = "Selected place",
                latitude = savedLat,
                longitude = savedLon,
                admin_level = "",
                country_code = null,
                display_name = ""
            )
            radiusSection.visibility = View.VISIBLE
            clearLocationButton.visibility = View.VISIBLE
        }

        selectedLabelId = args.getInt("label_id", -1).takeIf { it != -1 }

        // Date pickers
        startDateField.setOnClickListener {
            MaterialDatePicker.Builder.datePicker()
                .setTitleText("Start date")
                .apply { startDateMs?.let { setSelection(it) } }
                .build().also { picker ->
                    picker.addOnPositiveButtonClickListener { sel ->
                        startDateMs = sel
                        startDateField.setText(displayDateFormat.format(Date(sel)))
                        clearDatesButton.visibility = View.VISIBLE
                    }
                    picker.show(parentFragmentManager, "start_date")
                }
        }

        endDateField.setOnClickListener {
            MaterialDatePicker.Builder.datePicker()
                .setTitleText("End date")
                .apply { endDateMs?.let { setSelection(it) } }
                .build().also { picker ->
                    picker.addOnPositiveButtonClickListener { sel ->
                        endDateMs = sel
                        endDateField.setText(displayDateFormat.format(Date(sel)))
                        clearDatesButton.visibility = View.VISIBLE
                    }
                    picker.show(parentFragmentManager, "end_date")
                }
        }

        clearDatesButton.setOnClickListener {
            startDateMs = null
            endDateMs = null
            startDateField.text?.clear()
            endDateField.text?.clear()
            clearDatesButton.visibility = View.GONE
        }

        // Device & Label dropdowns
        val savedDeviceId = args.getString("device_id", "") ?: ""
        deviceDropdown.setAdapter(noFilterAdapter(listOf(ANY_DEVICE_LABEL)))
        deviceDropdown.setText(if (savedDeviceId.isNotEmpty()) savedDeviceId else ANY_DEVICE_LABEL, false)
        loadDevices(deviceDropdown, savedDeviceId)

        labelDropdown.setAdapter(noFilterAdapter(listOf(ALL_LABELS_LABEL)))
        labelDropdown.setText(ALL_LABELS_LABEL, false)
        labelDropdown.setOnItemClickListener { _, _, position, _ ->
            selectedLabelId = if (position <= 0) null else availableLabels.getOrNull(position - 1)?.id
        }
        loadLabels(labelDropdown)

        // Location autocomplete
        locationAdapter = object : ArrayAdapter<LocationResult>(
            requireContext(),
            android.R.layout.simple_dropdown_item_1line,
            locationSuggestions
        ) {
            override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
                val v = super.getView(position, convertView, parent)
                (v as? TextView)?.text = getItem(position)?.display_name ?: ""
                return v
            }

            override fun getFilter(): Filter = object : Filter() {
                override fun performFiltering(constraint: CharSequence?) = FilterResults().apply {
                    values = locationSuggestions.toList()
                    count = locationSuggestions.size
                }

                override fun publishResults(constraint: CharSequence?, results: FilterResults?) {
                    notifyDataSetChanged()
                }
            }
        }
        locationField.setAdapter(locationAdapter)
        locationField.threshold = 2

        locationField.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                locationSearchJob?.cancel()
                val query = s?.toString() ?: ""
                if (query.length >= 2) {
                    locationSearchJob = viewLifecycleOwner.lifecycleScope.launch {
                        delay(300)
                        searchPlaces(query, locationField)
                    }
                } else {
                    locationSuggestions.clear()
                    locationAdapter?.notifyDataSetChanged()
                    if (query.isEmpty()) {
                        selectedLocationResult = null
                        radiusSection.visibility = View.GONE
                        clearLocationButton.visibility = View.GONE
                    }
                }
            }

            override fun afterTextChanged(s: Editable?) {}
        })

        locationField.setOnItemClickListener { _, _, position, _ ->
            val selected = locationAdapter?.getItem(position)
            if (selected != null) {
                selectedLocationResult = selected
                locationField.setText(selected.display_name, false)
                radiusSection.visibility = View.VISIBLE
                clearLocationButton.visibility = View.VISIBLE
            }
        }

        clearLocationButton.setOnClickListener {
            selectedLocationResult = null
            locationField.text?.clear()
            radiusSection.visibility = View.GONE
            clearLocationButton.visibility = View.GONE
        }

        applyButton.setOnClickListener {
            val sortBy = when (sortChipGroup.checkedChipId) {
                R.id.sortSizeChip -> "size"
                R.id.sortQualityChip -> "quality"
                else -> "date"
            }
            val groupByPlace = groupByChipGroup.checkedChipId == R.id.groupByPlaceChip
            val mediaType = when (mediaTypeChipGroup.checkedChipId) {
                R.id.filterChipImages -> MediaTypeFilter.IMAGE
                R.id.filterChipVideos -> MediaTypeFilter.VIDEO
                else -> MediaTypeFilter.ALL
            }
            val deviceText = deviceDropdown.text.toString().trim()
            val deviceId = deviceText.takeIf { it.isNotEmpty() && it != ANY_DEVICE_LABEL }
            val filter = MediaFilter(
                mediaType = mediaType,
                starredOnly = starredSwitch.isChecked,
                startDate = startDateMs,
                endDate = endDateMs,
                labelId = selectedLabelId,
                deviceId = deviceId,
                locationLat = selectedLocationResult?.latitude,
                locationLon = selectedLocationResult?.longitude,
                locationRadiusKm = radiusSlider.value,
                minSimilarity = args.getFloat("min_similarity", 0.08f),
                sortBy = sortBy,
                sortOrder = sortOrderValue,
                groupByPlace = groupByPlace
            )
            (parentFragment as? OnFiltersApplied)?.onFiltersApplied(filter)
                ?: (activity as? OnFiltersApplied)?.onFiltersApplied(filter)
            dismiss()
        }

        clearButton.setOnClickListener {
            val defaultFilter = MediaFilter()
            (parentFragment as? OnFiltersApplied)?.onFiltersApplied(defaultFilter)
                ?: (activity as? OnFiltersApplied)?.onFiltersApplied(defaultFilter)
            dismiss()
        }
    }

    private fun updateSortOrderButtonText(button: MaterialButton) {
        button.text = if (sortOrderValue == "asc") "↑ Asc" else "↓ Desc"
    }

    private fun noFilterAdapter(items: List<String>): ArrayAdapter<String> {
        return object : ArrayAdapter<String>(
            requireContext(),
            android.R.layout.simple_dropdown_item_1line,
            items
        ) {
            override fun getFilter(): Filter = object : Filter() {
                override fun performFiltering(constraint: CharSequence?) = FilterResults().apply {
                    values = items
                    count = items.size
                }

                override fun publishResults(constraint: CharSequence?, results: FilterResults?) {
                    notifyDataSetChanged()
                }
            }
        }
    }

    private fun loadDevices(dropdown: MaterialAutoCompleteTextView, savedDeviceId: String) {
        viewLifecycleOwner.lifecycleScope.launch {
            val repo = RemoteMediaRepository(requireContext())
            repo.fetchDeviceIds().fold(
                onSuccess = { ids ->
                    val options = listOf(ANY_DEVICE_LABEL) + ids
                    dropdown.setAdapter(noFilterAdapter(options))
                    val display = if (savedDeviceId.isNotEmpty() && ids.contains(savedDeviceId)) {
                        savedDeviceId
                    } else {
                        ANY_DEVICE_LABEL
                    }
                    dropdown.setText(display, false)
                },
                onFailure = { /* keep default "Any device" option */ }
            )
        }
    }

    private fun loadLabels(dropdown: MaterialAutoCompleteTextView) {
        viewLifecycleOwner.lifecycleScope.launch {
            val repo = LabelRepository(requireContext())
            repo.fetchLabels().fold(
                onSuccess = { labels ->
                    availableLabels = labels
                    val options = listOf(ALL_LABELS_LABEL) + labels.map { it.name }
                    dropdown.setAdapter(noFilterAdapter(options))
                    val selectedName = labels.firstOrNull { it.id == selectedLabelId }?.name ?: ALL_LABELS_LABEL
                    dropdown.setText(selectedName, false)
                },
                onFailure = { /* keep default "All labels" option */ }
            )
        }
    }

    private fun searchPlaces(query: String, field: MaterialAutoCompleteTextView) {
        viewLifecycleOwner.lifecycleScope.launch {
            val repo = RemoteMediaRepository(requireContext())
            repo.searchPlaces(query).fold(
                onSuccess = { results ->
                    locationSuggestions.clear()
                    locationSuggestions.addAll(results)
                    locationAdapter?.notifyDataSetChanged()
                    if (results.isNotEmpty() && field.hasFocus()) field.showDropDown()
                },
                onFailure = { /* ignore */ }
            )
        }
    }
}
