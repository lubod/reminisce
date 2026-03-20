package org.openreminisce.app.fragments

import android.app.Dialog
import android.os.Bundle
import android.text.Editable
import android.text.TextWatcher
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ArrayAdapter
import android.widget.Filter
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.TextView
import com.google.android.material.bottomsheet.BottomSheetDialogFragment
import com.google.android.material.chip.Chip
import com.google.android.material.chip.ChipGroup
import com.google.android.material.datepicker.MaterialDatePicker
import com.google.android.material.slider.Slider
import com.google.android.material.switchmaterial.SwitchMaterial
import com.google.android.material.textfield.MaterialAutoCompleteTextView
import com.google.android.material.textfield.TextInputEditText
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.openreminisce.app.R
import org.openreminisce.app.model.Label
import org.openreminisce.app.model.LocationResult
import org.openreminisce.app.model.MediaFilter
import org.openreminisce.app.model.MediaTypeFilter
import org.openreminisce.app.model.SearchMode
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
        private const val ARG_CURRENT_SEARCH_MODE = "current_search_mode"

        fun newInstance(
            currentFilter: MediaFilter = MediaFilter(),
            showSearchMode: Boolean = false
        ) = FilterBottomSheetFragment().apply {
            arguments = Bundle().apply {
                putBoolean(ARG_SHOW_SEARCH_MODE, showSearchMode)
                putString(ARG_CURRENT_SEARCH_MODE, currentFilter.searchMode.name)
                // Pass current filter values for pre-population
                putLong("start_date", currentFilter.startDate ?: -1L)
                putLong("end_date", currentFilter.endDate ?: -1L)
                putBoolean("starred_only", currentFilter.starredOnly)
                putString("media_type", currentFilter.mediaType.name)
                putString("device_id", currentFilter.deviceId ?: "")
                putInt("label_id", currentFilter.labelId ?: -1)
                putDouble("lat", currentFilter.locationLat ?: Double.NaN)
                putDouble("lon", currentFilter.locationLon ?: Double.NaN)
                putFloat("radius_km", currentFilter.locationRadiusKm)
                putFloat("min_similarity", currentFilter.minSimilarity)
            }
        }
    }

    private val displayDateFormat = SimpleDateFormat("MMM d, yyyy", Locale.getDefault())

    private var startDateMs: Long? = null
    private var endDateMs: Long? = null
    private var selectedLabelId: Int? = null
    private var selectedLocationResult: LocationResult? = null
    private var locationSearchJob: Job? = null
    private var availableLabels: List<Label> = emptyList()
    private val locationSuggestions = mutableListOf<LocationResult>()
    private var locationAdapter: ArrayAdapter<LocationResult>? = null

    override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?): View? =
        inflater.inflate(R.layout.fragment_filter_bottom_sheet, container, false)

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        val startDateField = view.findViewById<TextInputEditText>(R.id.startDateField)
        val endDateField = view.findViewById<TextInputEditText>(R.id.endDateField)
        val starredSwitch = view.findViewById<SwitchMaterial>(R.id.starredSwitch)
        val mediaTypeChipGroup = view.findViewById<ChipGroup>(R.id.mediaTypeChipGroup)
        val deviceDropdown = view.findViewById<MaterialAutoCompleteTextView>(R.id.deviceDropdown)
        val labelChipGroup = view.findViewById<ChipGroup>(R.id.labelChipGroup)
        val locationField = view.findViewById<MaterialAutoCompleteTextView>(R.id.locationSearchField)
        val radiusSlider = view.findViewById<Slider>(R.id.locationRadiusSlider)
        val searchModeLabel = view.findViewById<TextView>(R.id.searchModeLabel)
        val searchModeGroup = view.findViewById<RadioGroup>(R.id.searchModeGroup)
        val applyButton = view.findViewById<View>(R.id.applyButton)
        val clearButton = view.findViewById<View>(R.id.clearButton)

        // Pre-populate from args
        val args = arguments ?: Bundle()
        val savedStartDate = args.getLong("start_date", -1L).takeIf { it != -1L }
        val savedEndDate = args.getLong("end_date", -1L).takeIf { it != -1L }
        startDateMs = savedStartDate
        endDateMs = savedEndDate
        savedStartDate?.let { startDateField.setText(displayDateFormat.format(Date(it))) }
        savedEndDate?.let { endDateField.setText(displayDateFormat.format(Date(it))) }

        starredSwitch.isChecked = args.getBoolean("starred_only", false)

        // Media type chips
        val savedMediaType = MediaTypeFilter.valueOf(args.getString("media_type", "ALL"))
        when (savedMediaType) {
            MediaTypeFilter.IMAGE -> mediaTypeChipGroup.check(R.id.filterChipImages)
            MediaTypeFilter.VIDEO -> mediaTypeChipGroup.check(R.id.filterChipVideos)
            else -> mediaTypeChipGroup.check(R.id.filterChipAll)
        }

        // Radius slider
        val savedRadius = args.getFloat("radius_km", 50f)
        radiusSlider.value = savedRadius.coerceIn(1f, 500f)

        // Search mode visibility
        val showSearchMode = args.getBoolean(ARG_SHOW_SEARCH_MODE, false)
        if (showSearchMode) {
            searchModeLabel.visibility = View.VISIBLE
            searchModeGroup.visibility = View.VISIBLE
            val savedMode = SearchMode.valueOf(args.getString(ARG_CURRENT_SEARCH_MODE, "SEMANTIC"))
            when (savedMode) {
                SearchMode.TEXT -> searchModeGroup.check(R.id.radioText)
                SearchMode.HYBRID -> searchModeGroup.check(R.id.radioHybrid)
                else -> searchModeGroup.check(R.id.radioSemantic)
            }
        }

        // Selected label id
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
                    }
                    picker.show(parentFragmentManager, "end_date")
                }
        }

        // Load devices and labels from API
        loadDevices(deviceDropdown, args.getString("device_id", ""))
        loadLabels(labelChipGroup)

        // Set up location adapter once — backed by mutable list updated on each server response.
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
            // Bypass built-in filtering — server already returned the right results.
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

        // Debounce: fetch suggestions after user pauses typing.
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
                }
            }
            override fun afterTextChanged(s: Editable?) {}
        })

        locationField.setOnItemClickListener { _, _, position, _ ->
            val selected = locationAdapter?.getItem(position)
            if (selected != null) {
                selectedLocationResult = selected
                locationField.setText(selected.display_name, false)
            }
        }

        applyButton.setOnClickListener {
            val mediaType = when (mediaTypeChipGroup.checkedChipId) {
                R.id.filterChipImages -> MediaTypeFilter.IMAGE
                R.id.filterChipVideos -> MediaTypeFilter.VIDEO
                else -> MediaTypeFilter.ALL
            }
            val searchMode = when (searchModeGroup.checkedRadioButtonId) {
                R.id.radioText -> SearchMode.TEXT
                R.id.radioHybrid -> SearchMode.HYBRID
                else -> SearchMode.SEMANTIC
            }
            val deviceId = deviceDropdown.text.toString().trim().takeIf { it.isNotEmpty() }
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
                searchMode = searchMode,
                minSimilarity = args.getFloat("min_similarity", 0.08f)
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

    private fun loadDevices(dropdown: MaterialAutoCompleteTextView, savedDeviceId: String) {
        viewLifecycleOwner.lifecycleScope.launch {
            val repo = RemoteMediaRepository(requireContext())
            repo.fetchDeviceIds().fold(
                onSuccess = { ids ->
                    val adapter = ArrayAdapter(requireContext(), android.R.layout.simple_dropdown_item_1line, ids)
                    dropdown.setAdapter(adapter)
                    if (savedDeviceId.isNotEmpty()) dropdown.setText(savedDeviceId, false)
                },
                onFailure = { /* ignore */ }
            )
        }
    }

    private fun loadLabels(chipGroup: ChipGroup) {
        viewLifecycleOwner.lifecycleScope.launch {
            val repo = LabelRepository(requireContext())
            repo.fetchLabels().fold(
                onSuccess = { labels ->
                    availableLabels = labels
                    chipGroup.removeAllViews()
                    labels.forEach { label ->
                        val chip = Chip(requireContext()).apply {
                            text = label.name
                            isCheckable = true
                            isChecked = label.id == selectedLabelId
                            setOnCheckedChangeListener { _, checked ->
                                selectedLabelId = if (checked) label.id else null
                            }
                        }
                        chipGroup.addView(chip)
                    }
                },
                onFailure = { /* ignore */ }
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
