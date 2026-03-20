package org.openreminisce.app.model

data class LocationResult(
    val name: String,
    val latitude: Double,
    val longitude: Double,
    val admin_level: String,
    val country_code: String? = null,
    val display_name: String
)
