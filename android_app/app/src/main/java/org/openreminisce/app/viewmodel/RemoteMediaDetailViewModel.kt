package org.openreminisce.app.viewmodel

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.openreminisce.app.model.ImageMetadata
import org.openreminisce.app.model.Label
import org.openreminisce.app.repository.LabelRepository
import org.openreminisce.app.repository.RemoteMediaRepository

class RemoteMediaDetailViewModel(context: Context) : ViewModel() {

    private val repository = RemoteMediaRepository(context)
    private val labelRepository = LabelRepository(context)

    private val _metadata = MutableStateFlow<ImageMetadata?>(null)
    val metadata: StateFlow<ImageMetadata?> = _metadata.asStateFlow()

    private val _mediaLabels = MutableStateFlow<List<Label>>(emptyList())
    val mediaLabels: StateFlow<List<Label>> = _mediaLabels.asStateFlow()

    private val _allLabels = MutableStateFlow<List<Label>>(emptyList())
    val allLabels: StateFlow<List<Label>> = _allLabels.asStateFlow()

    private val _isStarred = MutableStateFlow(false)
    val isStarred: StateFlow<Boolean> = _isStarred.asStateFlow()

    private val _isLoading = MutableStateFlow(false)
    val isLoading: StateFlow<Boolean> = _isLoading.asStateFlow()

    private val _error = MutableSharedFlow<String>()
    val error: SharedFlow<String> = _error.asSharedFlow()

    /** Emits (hash, newStarredState) after a successful star toggle. */
    private val _starToggleResult = MutableSharedFlow<Pair<String, Boolean>>()
    val starToggleResult: SharedFlow<Pair<String, Boolean>> = _starToggleResult.asSharedFlow()

    /** Emits Result<Unit> after a delete attempt. */
    private val _deleteResult = MutableSharedFlow<Result<Unit>>()
    val deleteResult: SharedFlow<Result<Unit>> = _deleteResult.asSharedFlow()

    fun loadMetadata(hash: String) {
        viewModelScope.launch {
            _isLoading.value = true
            repository.fetchMetadata(hash).fold(
                onSuccess = { meta ->
                    _metadata.value = meta
                    _isStarred.value = meta.starred
                },
                onFailure = { _error.emit(it.message ?: "Failed to load metadata") }
            )
            _isLoading.value = false
        }
    }

    fun toggleStar(hash: String, mediaType: String) {
        val optimistic = !_isStarred.value
        _isStarred.value = optimistic          // immediate UI update
        viewModelScope.launch {
            repository.toggleStar(hash, mediaType).fold(
                onSuccess = { r ->
                    _isStarred.value = r.starred
                    _metadata.value = _metadata.value?.copy(starred = r.starred)
                    _starToggleResult.emit(hash to r.starred)
                },
                onFailure = {
                    _isStarred.value = !optimistic   // revert
                    _error.emit(it.message ?: "Failed to toggle star")
                }
            )
        }
    }

    fun deleteMedia(hash: String, mediaType: String) {
        viewModelScope.launch {
            val result = repository.deleteMedia(hash, mediaType)
            _deleteResult.emit(result)
            if (result.isFailure) {
                _error.emit(result.exceptionOrNull()?.message ?: "Failed to delete media")
            }
        }
    }

    fun loadAllLabels() {
        viewModelScope.launch {
            labelRepository.fetchLabels().fold(
                onSuccess = { _allLabels.value = it },
                onFailure = { /* silent: labels panel shows empty */ }
            )
        }
    }

    fun loadMediaLabels(hash: String, mediaType: String) {
        viewModelScope.launch {
            labelRepository.getMediaLabels(hash, mediaType).fold(
                onSuccess = { _mediaLabels.value = it },
                onFailure = { /* silent */ }
            )
        }
    }

    fun addLabelToMedia(hash: String, mediaType: String, labelId: Int) {
        viewModelScope.launch {
            labelRepository.addLabelToMedia(hash, mediaType, labelId).fold(
                onSuccess = { loadMediaLabels(hash, mediaType) },
                onFailure = { _error.emit(it.message ?: "Failed to add label") }
            )
        }
    }

    fun removeLabelFromMedia(hash: String, mediaType: String, labelId: Int) {
        viewModelScope.launch {
            labelRepository.removeLabelFromMedia(hash, mediaType, labelId).fold(
                onSuccess = { loadMediaLabels(hash, mediaType) },
                onFailure = { _error.emit(it.message ?: "Failed to remove label") }
            )
        }
    }

    fun createLabel(name: String, color: String) {
        viewModelScope.launch {
            labelRepository.createLabel(name, color).fold(
                onSuccess = { loadAllLabels() },
                onFailure = { _error.emit(it.message ?: "Failed to create label") }
            )
        }
    }

    companion object {
        fun factory(context: Context) = object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T =
                RemoteMediaDetailViewModel(context.applicationContext) as T
        }
    }
}
