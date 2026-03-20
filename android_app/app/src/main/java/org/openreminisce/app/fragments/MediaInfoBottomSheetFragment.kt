package org.openreminisce.app.fragments

import android.graphics.Color
import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.TextView
import androidx.activity.viewModels
import androidx.appcompat.app.AlertDialog
import androidx.fragment.app.Fragment
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import androidx.viewpager2.adapter.FragmentStateAdapter
import androidx.viewpager2.widget.ViewPager2
import com.google.android.material.bottomsheet.BottomSheetDialogFragment
import com.google.android.material.chip.Chip
import com.google.android.material.chip.ChipGroup
import com.google.android.material.tabs.TabLayout
import com.google.android.material.tabs.TabLayoutMediator
import io.noties.markwon.Markwon
import kotlinx.coroutines.launch
import org.json.JSONObject
import org.openreminisce.app.R
import org.openreminisce.app.model.Label
import org.openreminisce.app.viewmodel.RemoteMediaDetailViewModel

class MediaInfoBottomSheetFragment : BottomSheetDialogFragment() {

    companion object {
        private const val ARG_HASH = "hash"
        private const val ARG_MEDIA_TYPE = "media_type"

        fun newInstance(hash: String, mediaType: String) = MediaInfoBottomSheetFragment().apply {
            arguments = Bundle().apply {
                putString(ARG_HASH, hash)
                putString(ARG_MEDIA_TYPE, mediaType)
            }
        }

        private val TAB_TITLES = listOf("Details", "EXIF", "Labels", "Description")
    }

    private val hash get() = arguments?.getString(ARG_HASH) ?: ""
    private val mediaType get() = arguments?.getString(ARG_MEDIA_TYPE) ?: "image"

    override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?): View? =
        inflater.inflate(R.layout.fragment_media_info, container, false)

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        val tabLayout = view.findViewById<TabLayout>(R.id.mediaInfoTabLayout)
        val viewPager = view.findViewById<ViewPager2>(R.id.mediaInfoViewPager)

        viewPager.adapter = InfoPagerAdapter(requireActivity())
        TabLayoutMediator(tabLayout, viewPager) { tab, position ->
            tab.text = TAB_TITLES[position]
        }.attach()
    }

    private inner class InfoPagerAdapter(activity: FragmentActivity) : FragmentStateAdapter(activity) {
        override fun getItemCount() = 4
        override fun createFragment(position: Int): Fragment = when (position) {
            0 -> DetailsTabFragment.newInstance(hash, mediaType)
            1 -> ExifTabFragment.newInstance(hash, mediaType)
            2 -> LabelsTabFragment.newInstance(hash, mediaType)
            else -> DescriptionTabFragment.newInstance(hash, mediaType)
        }
    }

    // ── Tab fragments ────────────────────────────────────────────────────────

    class DetailsTabFragment : Fragment() {
        companion object {
            fun newInstance(hash: String, mediaType: String) = DetailsTabFragment().apply {
                arguments = Bundle().apply { putString("hash", hash); putString("media_type", mediaType) }
            }
        }

        private fun viewModel(): RemoteMediaDetailViewModel {
            val activity = requireActivity()
            val factory = RemoteMediaDetailViewModel.factory(activity.applicationContext)
            return androidx.lifecycle.ViewModelProvider(activity, factory)[RemoteMediaDetailViewModel::class.java]
        }

        override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?): View? =
            inflater.inflate(R.layout.tab_details, container, false)

        override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
            val fileNameText = view.findViewById<TextView>(R.id.detailFileName)
            val dateText = view.findViewById<TextView>(R.id.detailDate)
            val placeText = view.findViewById<TextView>(R.id.detailPlace)
            val deviceIdText = view.findViewById<TextView>(R.id.detailDeviceId)

            lifecycleScope.launch {
                viewModel().metadata.collect { meta ->
                    if (meta != null) {
                        fileNameText.text = "File: ${meta.name.ifEmpty { meta.hash.takeLast(8) }}"
                        dateText.text = "Date: ${meta.created_at}"
                        if (!meta.place.isNullOrEmpty()) {
                            placeText.text = "Place: ${meta.place}"
                            placeText.visibility = View.VISIBLE
                        } else {
                            placeText.visibility = View.GONE
                        }
                        if (!meta.device_id.isNullOrEmpty()) {
                            deviceIdText.text = "Device: ${meta.device_id}"
                            deviceIdText.visibility = View.VISIBLE
                        } else {
                            deviceIdText.visibility = View.GONE
                        }
                    }
                }
            }
        }
    }

    class ExifTabFragment : Fragment() {
        companion object {
            fun newInstance(hash: String, mediaType: String) = ExifTabFragment().apply {
                arguments = Bundle().apply { putString("hash", hash); putString("media_type", mediaType) }
            }
        }

        private fun viewModel(): RemoteMediaDetailViewModel {
            val activity = requireActivity()
            val factory = RemoteMediaDetailViewModel.factory(activity.applicationContext)
            return androidx.lifecycle.ViewModelProvider(activity, factory)[RemoteMediaDetailViewModel::class.java]
        }

        override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?): View? =
            inflater.inflate(R.layout.tab_exif, container, false)

        override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
            val recyclerView = view.findViewById<RecyclerView>(R.id.exifRecyclerView)
            recyclerView.layoutManager = LinearLayoutManager(requireContext())

            lifecycleScope.launch {
                viewModel().metadata.collect { meta ->
                    val exifJson = meta?.exif
                    val formattedPairs = if (!exifJson.isNullOrEmpty()) formatExifData(exifJson) else emptyList()
                    recyclerView.adapter = ExifAdapter(formattedPairs, exifJson)
                }
            }
        }

        /** Format EXIF JSON into human-readable key-value pairs, matching web client logic. */
        private fun formatExifData(json: String): List<Pair<String, String>> {
            val result = mutableListOf<Pair<String, String>>()
            return try {
                val obj = JSONObject(json)

                fun opt(vararg keys: String): String? {
                    for (k in keys) {
                        val v = obj.opt(k)
                        if (v != null && v.toString().isNotEmpty() && v.toString() != "null") return v.toString()
                    }
                    return null
                }

                // Camera
                val make = opt("Make")
                val model = opt("Model")
                when {
                    make != null && model != null -> result.add("Camera" to "$make $model")
                    model != null -> result.add("Camera" to model)
                    make != null -> result.add("Camera" to make)
                }

                // Lens
                opt("LensModel", "LensInfo")?.let { result.add("Lens" to it) }

                // Shutter speed
                opt("ExposureTime")?.let { raw ->
                    val formatted = try {
                        val parts = raw.split("/")
                        if (parts.size == 2) {
                            val num = parts[0].trim().toDouble()
                            val den = parts[1].trim().toDouble()
                            if (num == 1.0 || num < 1.0) raw
                            else "1/${(den / num).toInt()}"
                        } else {
                            val v = raw.toDouble()
                            if (v >= 1) "${v}s" else "1/${(1 / v).toInt()}s"
                        }
                    } catch (_: Exception) { raw }
                    result.add("Shutter Speed" to formatted)
                }

                // Aperture
                opt("FNumber", "ApertureValue")?.let { raw ->
                    val formatted = try {
                        val v = raw.toDouble()
                        "f/${String.format("%.1f", v)}"
                    } catch (_: Exception) { raw }
                    result.add("Aperture" to formatted)
                }

                // ISO
                opt("ISO", "ISOSpeedRatings", "PhotographicSensitivity")?.let {
                    result.add("ISO" to it)
                }

                // Focal length
                opt("FocalLength")?.let { raw ->
                    val formatted = try {
                        "${raw.toDouble().toInt()}mm"
                    } catch (_: Exception) { raw }
                    result.add("Focal Length" to formatted)
                }

                // 35mm equivalent
                opt("FocalLengthIn35mmFilm", "FocalLengthIn35mmFormat")?.let { raw ->
                    val formatted = try { "${raw.toDouble().toInt()}mm" } catch (_: Exception) { raw }
                    result.add("35mm Equiv." to formatted)
                }

                // Resolution
                val w = opt("PixelXDimension", "ImageWidth")
                val h = opt("PixelYDimension", "ImageLength", "ImageHeight")
                if (w != null && h != null) result.add("Resolution" to "${w} × ${h}")

                // Orientation
                opt("Orientation")?.let { raw ->
                    val label = when (raw.trim()) {
                        "1" -> "Normal"
                        "3" -> "Rotated 180°"
                        "6" -> "Rotated 90° CW"
                        "8" -> "Rotated 90° CCW"
                        else -> raw
                    }
                    result.add("Orientation" to label)
                }

                // Date taken
                opt("DateTimeOriginal", "DateTime", "DateTimeDigitized")?.let {
                    result.add("Date Taken" to it)
                }

                // GPS
                val lat = opt("GPSLatitude")
                val lon = opt("GPSLongitude")
                if (lat != null && lon != null) result.add("GPS" to "$lat, $lon")

                // Flash
                opt("Flash")?.let { raw ->
                    val fired = try { raw.toInt() and 0x01 != 0 } catch (_: Exception) { false }
                    result.add("Flash" to if (fired) "Fired" else "Did not fire")
                }

                // White Balance
                opt("WhiteBalance")?.let { raw ->
                    result.add("White Balance" to when (raw.trim()) { "0" -> "Auto"; "1" -> "Manual"; else -> raw })
                }

                // Exposure Mode
                opt("ExposureMode")?.let { raw ->
                    result.add("Exposure Mode" to when (raw.trim()) { "0" -> "Auto"; "1" -> "Manual"; "2" -> "Auto bracket"; else -> raw })
                }

                // Metering Mode
                opt("MeteringMode")?.let { raw ->
                    val label = when (raw.trim()) {
                        "0" -> "Unknown"; "1" -> "Average"; "2" -> "Center-weighted"; "3" -> "Spot"
                        "4" -> "Multi-spot"; "5" -> "Pattern"; "6" -> "Partial"; else -> raw
                    }
                    result.add("Metering Mode" to label)
                }

                result
            } catch (e: Exception) {
                emptyList()
            }
        }

        private inner class ExifAdapter(
            private val items: List<Pair<String, String>>,
            private val rawJson: String?
        ) : RecyclerView.Adapter<RecyclerView.ViewHolder>() {

            private val TYPE_PAIR = 0
            private val TYPE_RAW_TOGGLE = 1

            private var rawExpanded = false

            inner class PairVH(view: View) : RecyclerView.ViewHolder(view) {
                val keyView: TextView = view.findViewById(R.id.exifKey)
                val valueView: TextView = view.findViewById(R.id.exifValue)
            }

            inner class RawVH(view: View) : RecyclerView.ViewHolder(view) {
                val toggleButton: Button = view.findViewById(R.id.rawExifToggle)
                val rawText: TextView = view.findViewById(R.id.rawExifText)
            }

            override fun getItemCount() = if (rawJson.isNullOrEmpty()) items.size else items.size + 1

            override fun getItemViewType(position: Int) =
                if (position < items.size) TYPE_PAIR else TYPE_RAW_TOGGLE

            override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): RecyclerView.ViewHolder {
                return if (viewType == TYPE_PAIR) {
                    PairVH(LayoutInflater.from(parent.context).inflate(R.layout.item_exif_pair, parent, false))
                } else {
                    RawVH(LayoutInflater.from(parent.context).inflate(R.layout.item_exif_raw_toggle, parent, false))
                }
            }

            override fun onBindViewHolder(holder: RecyclerView.ViewHolder, position: Int) {
                if (holder is PairVH) {
                    holder.keyView.text = items[position].first
                    holder.valueView.text = items[position].second
                } else if (holder is RawVH) {
                    holder.rawText.text = rawJson
                    holder.rawText.visibility = if (rawExpanded) View.VISIBLE else View.GONE
                    holder.toggleButton.text = if (rawExpanded) "Hide raw EXIF" else "Show raw EXIF"
                    holder.toggleButton.setOnClickListener {
                        rawExpanded = !rawExpanded
                        notifyItemChanged(position)
                    }
                }
            }
        }
    }

    class LabelsTabFragment : Fragment() {
        companion object {
            fun newInstance(hash: String, mediaType: String) = LabelsTabFragment().apply {
                arguments = Bundle().apply { putString("hash", hash); putString("media_type", mediaType) }
            }
        }

        private val hash get() = arguments?.getString("hash") ?: ""
        private val mediaType get() = arguments?.getString("media_type") ?: "image"

        private fun viewModel(): RemoteMediaDetailViewModel {
            val activity = requireActivity()
            val factory = RemoteMediaDetailViewModel.factory(activity.applicationContext)
            return androidx.lifecycle.ViewModelProvider(activity, factory)[RemoteMediaDetailViewModel::class.java]
        }

        override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?): View? =
            inflater.inflate(R.layout.tab_labels, container, false)

        override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
            val chipGroup = view.findViewById<ChipGroup>(R.id.labelsChipGroup)
            val addButton = view.findViewById<Button>(R.id.addLabelButton)
            val vm = viewModel()

            lifecycleScope.launch {
                vm.mediaLabels.collect { labels ->
                    rebuildChips(chipGroup, labels, vm)
                }
            }

            addButton.setOnClickListener {
                showAddLabelDialog(vm)
            }
        }

        private fun rebuildChips(chipGroup: ChipGroup, labels: List<Label>, vm: RemoteMediaDetailViewModel) {
            chipGroup.removeAllViews()
            labels.forEach { label ->
                val chip = Chip(requireContext()).apply {
                    text = label.name
                    isCloseIconVisible = true
                    try { setChipBackgroundColorResource(android.R.color.transparent)
                        chipStrokeWidth = 2f
                        setChipStrokeColor(android.content.res.ColorStateList.valueOf(Color.parseColor(label.color)))
                    } catch (_: Exception) { }
                    setOnCloseIconClickListener {
                        vm.removeLabelFromMedia(hash, mediaType, label.id)
                    }
                }
                chipGroup.addView(chip)
            }
        }

        private fun showAddLabelDialog(vm: RemoteMediaDetailViewModel) {
            val allLabels = vm.allLabels.value
            if (allLabels.isEmpty()) {
                showCreateLabelDialog(vm)
                return
            }

            val names = allLabels.map { it.name }.toMutableList().also { it.add("+ Create new label") }
            AlertDialog.Builder(requireContext())
                .setTitle("Add label")
                .setItems(names.toTypedArray()) { _, which ->
                    if (which == allLabels.size) {
                        showCreateLabelDialog(vm)
                    } else {
                        vm.addLabelToMedia(hash, mediaType, allLabels[which].id)
                    }
                }
                .show()
        }

        private fun showCreateLabelDialog(vm: RemoteMediaDetailViewModel) {
            val colors = listOf("#FF5252", "#FF6D00", "#FFD740", "#69F0AE", "#40C4FF",
                "#7C4DFF", "#E040FB", "#FF4081", "#80CBC4", "#B0BEC5")

            val editText = android.widget.EditText(requireContext()).apply {
                hint = "Label name"
                setPadding(48, 24, 48, 24)
            }

            AlertDialog.Builder(requireContext())
                .setTitle("New label")
                .setView(editText)
                .setPositiveButton("Create") { _, _ ->
                    val name = editText.text.toString().trim()
                    if (name.isNotEmpty()) {
                        val color = colors.random()
                        vm.createLabel(name, color)
                    }
                }
                .setNegativeButton("Cancel", null)
                .show()
        }
    }

    class DescriptionTabFragment : Fragment() {
        companion object {
            fun newInstance(hash: String, mediaType: String) = DescriptionTabFragment().apply {
                arguments = Bundle().apply { putString("hash", hash); putString("media_type", mediaType) }
            }
        }

        private fun viewModel(): RemoteMediaDetailViewModel {
            val activity = requireActivity()
            val factory = RemoteMediaDetailViewModel.factory(activity.applicationContext)
            return androidx.lifecycle.ViewModelProvider(activity, factory)[RemoteMediaDetailViewModel::class.java]
        }

        override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?): View? =
            inflater.inflate(R.layout.tab_description, container, false)

        override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
            val descText = view.findViewById<TextView>(R.id.descriptionText)
            val markwon = Markwon.create(requireContext())
            lifecycleScope.launch {
                viewModel().metadata.collect { meta ->
                    val text = meta?.description?.takeIf { it.isNotEmpty() } ?: "No description available"
                    markwon.setMarkdown(descText, text)
                }
            }
        }
    }
}
