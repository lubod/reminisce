package org.openreminisce.app.model

data class Label(
    val id: Int,
    val name: String,
    val color: String,
    val created_at: String
)

data class LabelsResponse(val labels: List<Label>)

data class StarResponse(val hash: String, val starred: Boolean)
