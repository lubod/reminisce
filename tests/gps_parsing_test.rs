use serde_json::json;

#[cfg(test)]
mod gps_parsing_tests {
    use super::*;

    #[test]
    fn test_extract_gps_coordinates_rational_format() {
        // Test with rational fraction format (e.g., "37/1, 25/1, 1919/100 N")
        // This is the actual format from the log that was failing
        let exif_json = json!({
            "GPSLatitude": "37/1, 25/1, 1919/100 N",
            "GPSLongitude": "122/1, 5/1, 240/100 W",
            "GPSLatitudeRef": "N",
            "GPSLongitudeRef": "W"
        });

        let result = reminisce::utils::extract_gps_coordinates(&exif_json);
        assert!(result.is_some(), "GPS coordinates should be extracted");

        let (lat, lon) = result.unwrap();

        // Expected latitude: 37 + 25/60 + 19.19/3600 = 37.421941666...
        assert!((lat - 37.421941).abs() < 0.0001, "Latitude should be ~37.4219, got {}", lat);

        // Expected longitude: -(122 + 5/60 + 2.4/3600) = -122.084
        assert!((lon - (-122.084)).abs() < 0.001, "Longitude should be ~-122.084, got {}", lon);
    }

    #[test]
    fn test_extract_gps_coordinates_traditional_format() {
        // Test with traditional degree/minute/second format
        let exif_json = json!({
            "GPSLatitude": "52 deg 31 min 1.20 sec",
            "GPSLongitude": "13 deg 24 min 15.60 sec",
            "GPSLatitudeRef": "N",
            "GPSLongitudeRef": "E"
        });

        let result = reminisce::utils::extract_gps_coordinates(&exif_json);
        assert!(result.is_some(), "GPS coordinates should be extracted");

        let (lat, lon) = result.unwrap();

        // Expected latitude: 52 + 31/60 + 1.20/3600 = 52.517
        assert!((lat - 52.517).abs() < 0.001, "Latitude should be ~52.517, got {}", lat);

        // Expected longitude: 13 + 24/60 + 15.60/3600 = 13.404333
        assert!((lon - 13.404333).abs() < 0.001, "Longitude should be ~13.404, got {}", lon);
    }

    #[test]
    fn test_extract_gps_coordinates_southern_hemisphere() {
        // Test with southern hemisphere coordinates
        let exif_json = json!({
            "GPSLatitude": "33/1, 52/1, 800/100 S",
            "GPSLongitude": "151/1, 12/1, 4500/100 E",
            "GPSLatitudeRef": "S",
            "GPSLongitudeRef": "E"
        });

        let result = reminisce::utils::extract_gps_coordinates(&exif_json);
        assert!(result.is_some(), "GPS coordinates should be extracted");

        let (lat, lon) = result.unwrap();

        // Expected latitude: -(33 + 52/60 + 8/3600) = -33.868888
        assert!((lat - (-33.868888)).abs() < 0.0001, "Latitude should be negative (S), got {}", lat);
        assert!(lat < 0.0, "Southern latitude should be negative");

        // Expected longitude: 151 + 12/60 + 45/3600 = 151.2125
        assert!((lon - 151.2125).abs() < 0.001, "Longitude should be positive (E), got {}", lon);
        assert!(lon > 0.0, "Eastern longitude should be positive");
    }

    #[test]
    fn test_extract_gps_coordinates_missing_fields() {
        // Test with missing GPS fields
        let exif_json = json!({
            "GPSLatitude": "37/1, 25/1, 1919/100 N",
            "GPSLatitudeRef": "N"
            // Missing longitude fields
        });

        let result = reminisce::utils::extract_gps_coordinates(&exif_json);
        assert!(result.is_none(), "Should return None when GPS fields are missing");
    }

    #[test]
    fn test_extract_gps_coordinates_no_gps_data() {
        // Test with no GPS data at all
        let exif_json = json!({
            "Make": "Google",
            "Model": "Pixel 6",
            "DateTime": "2025-10-22 07:22:36"
        });

        let result = reminisce::utils::extract_gps_coordinates(&exif_json);
        assert!(result.is_none(), "Should return None when no GPS data present");
    }

    #[test]
    fn test_extract_gps_coordinates_western_hemisphere() {
        // Test with western hemisphere (negative longitude)
        let exif_json = json!({
            "GPSLatitude": "40/1, 44/1, 5460/100 N",
            "GPSLongitude": "73/1, 59/1, 2400/100 W",
            "GPSLatitudeRef": "N",
            "GPSLongitudeRef": "W"
        });

        let result = reminisce::utils::extract_gps_coordinates(&exif_json);
        assert!(result.is_some(), "GPS coordinates should be extracted");

        let (lat, lon) = result.unwrap();

        // New York City coordinates
        // Expected: 40 + 44/60 + 54.60/3600 = 40.7485
        assert!((lat - 40.7485).abs() < 0.001, "Latitude should be ~40.7485, got {}", lat);
        // Expected: -(73 + 59/60 + 24/3600) = -73.99
        assert!((lon - (-73.99)).abs() < 0.01, "Longitude should be negative (W), got {}", lon);
        assert!(lon < 0.0, "Western longitude should be negative");
    }

    #[test]
    fn test_extract_gps_coordinates_zero_seconds() {
        // Test with zero seconds
        let exif_json = json!({
            "GPSLatitude": "45/1, 30/1, 0/1 N",
            "GPSLongitude": "90/1, 15/1, 0/1 E",
            "GPSLatitudeRef": "N",
            "GPSLongitudeRef": "E"
        });

        let result = reminisce::utils::extract_gps_coordinates(&exif_json);
        assert!(result.is_some(), "GPS coordinates should be extracted");

        let (lat, lon) = result.unwrap();

        // Expected: 45.5 and 90.25
        assert!((lat - 45.5).abs() < 0.0001, "Latitude should be 45.5, got {}", lat);
        assert!((lon - 90.25).abs() < 0.0001, "Longitude should be 90.25, got {}", lon);
    }
}
