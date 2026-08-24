//! Query builder utilities for constructing SQL queries dynamically
//!
//! This module provides utilities to safely build SQL queries with dynamic
//! WHERE conditions and parameters, avoiding string concatenation errors.

use crate::constants::tables;
use crate::utils;

/// Constant-safe query returned by build methods when the configured table
/// failed whitelist validation: evaluates to zero rows instead of interpolating
/// an unvalidated table name into SQL.
const SAFE_FALLBACK_QUERY: &str = "SELECT 1 WHERE false";

/// Represents a SQL query parameter position and value type
pub struct QueryParam {
    pub position: usize,
}

/// Builder for media listing queries (images/videos with thumbnails)
pub struct MediaQueryBuilder {
    table: String,
    conditions: Vec<String>,
    param_count: usize,
    user_id_param: Option<usize>,
    label_id_param: Option<usize>,
    table_validated: bool,
}

impl MediaQueryBuilder {
    /// Create a new query builder for the given table. The table is validated
    /// against the strict whitelist (`utils::validate_table_name`); if it fails,
    /// the error is logged once here and every build_* method returns the
    /// constant-safe fallback query instead of interpolated SQL.
    pub fn new(table: &str) -> Self {
        let table_validated = match utils::validate_table_name(table) {
            Ok(()) => true,
            Err(reason) => {
                log::error!(
                    "Query builder rejected table '{}': {}",
                    table, reason
                );
                false
            }
        };
        Self {
            table: table.to_string(),
            conditions: vec!["t.deleted_at IS NULL".to_string()],
            param_count: 0,
            user_id_param: None,
            label_id_param: None,
            table_validated,
        }
    }

    /// True when the configured table passed whitelist validation.
    pub fn table_validated(&self) -> bool {
        self.table_validated
    }

    /// Set the user_id parameter for starred images JOIN
    pub fn with_user_id(&mut self) -> QueryParam {
        self.param_count += 1;
        self.user_id_param = Some(self.param_count);
        QueryParam { position: self.param_count }
    }

    /// Add device ID filter condition (for optional admin device filtering)
    pub fn with_device_id(&mut self) -> QueryParam {
        self.param_count += 1;
        self.conditions.push(format!("t.deviceid = ${}", self.param_count));
        QueryParam { position: self.param_count }
    }

    /// Add user_id filter condition for access control (non-admin users)
    /// Uses the same parameter position as with_user_id() since user_id is the same value
    pub fn with_user_id_filter(&mut self) {
        if let Some(user_id_param) = self.user_id_param {
            self.conditions.push(format!("t.user_id = ${}", user_id_param));
        }
    }

    /// Add starred-only filter condition
    pub fn with_starred_only(&mut self) {
        self.conditions.push("s.hash IS NOT NULL".to_string());
    }

    /// Add label ID filter condition
    pub fn with_label_id(&mut self) -> QueryParam {
        self.param_count += 1;
        self.label_id_param = Some(self.param_count);
        QueryParam { position: self.param_count }
    }

    /// Add has_thumbnail filter (always included for thumbnail queries)
    pub fn with_has_thumbnail(&mut self) {
        self.conditions.push("t.has_thumbnail = true".to_string());
    }

    /// Only include images that carry no EXIF metadata at all (no EXIF orientation available).
    /// Used by the "Orientation check" tab: these photos rely on AI orientation detection
    /// and may need manual review. Images table only.
    pub fn with_no_exif(&mut self) {
        self.conditions.push("t.exif IS NULL".to_string());
    }

    /// True when this builder targets the images table (only images have an `exif` column).
    pub fn is_images_table(&self) -> bool {
        self.table == tables::IMAGES
    }

    /// True when this builder targets the videos table.
    pub fn is_videos_table(&self) -> bool {
        self.table == tables::VIDEOS
    }

    /// Add start date filter condition (created_at >= start_date)
    pub fn with_start_date(&mut self) -> QueryParam {
        self.param_count += 1;
        self.conditions.push(format!("t.created_at >= ${}", self.param_count));
        QueryParam { position: self.param_count }
    }

    /// Add end date filter condition (created_at < end_date + 1 day)
    pub fn with_end_date(&mut self) -> QueryParam {
        self.param_count += 1;
        self.conditions.push(format!("t.created_at < ${}", self.param_count));
        QueryParam { position: self.param_count }
    }

    /// Add a custom condition (for complex filters like PostGIS queries)
    pub fn add_custom_condition(&mut self, condition: String) {
        self.conditions.push(condition);
    }

    /// Build the WHERE clause
    fn build_where_clause(&self) -> String {
        if self.conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.conditions.join(" AND "))
        }
    }

    /// Build SELECT query body (SELECT ... FROM ... JOIN ... WHERE ...)
    /// Returns the query string without ORDER BY, LIMIT, and OFFSET
    pub fn build_select_body(&self, lon_param: Option<usize>, lat_param: Option<usize>) -> String {
        if !self.table_validated {
            return SAFE_FALLBACK_QUERY.to_string();
        }
        let has_location = lon_param.is_some() && lat_param.is_some();

        let select_clause = if self.table == tables::IMAGES {
            if has_location {
                let lon_p = lon_param.unwrap();
                let lat_p = lat_param.unwrap();
                format!(
                    "SELECT t.hash, t.name, t.created_at, t.place, t.deviceid, \
                     CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred, \
                     ST_Distance(t.location, ST_MakePoint(${}, ${})::geography) / 1000.0 as distance_km, \
                     'image' as media_type, t.file_size_bytes::bigint as file_size_bytes, \
                     t.aesthetic_score",
                    lon_p, lat_p
                )
            } else {
                "SELECT t.hash, t.name, t.created_at, t.place, t.deviceid, \
                 CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred, \
                 NULL::double precision as distance_km, \
                 'image' as media_type, t.file_size_bytes::bigint as file_size_bytes, \
                 t.aesthetic_score".to_string()
            }
        } else {
            "SELECT t.hash, t.name, t.created_at, NULL as place, t.deviceid, \
             CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred, \
             NULL::double precision as distance_km, \
             'video' as media_type, t.file_size_bytes as file_size_bytes, \
             NULL::real as aesthetic_score".to_string()
        };

        let where_clause = self.build_where_clause();

        // Use appropriate starred table based on media type
        let starred_table = if self.table == tables::IMAGES {
            tables::STARRED_IMAGES
        } else {
            tables::STARRED_VIDEOS
        };

        let join_clause = if let Some(user_id_param) = self.user_id_param {
            format!("LEFT JOIN {} s ON t.hash = s.hash AND s.user_id = ${}", starred_table, user_id_param)
        } else {
            format!("LEFT JOIN {} s ON t.hash = s.hash", starred_table)
        };

        // Add label filtering join if needed
        let label_join_clause = if let Some(label_id_param) = self.label_id_param {
            let label_table = if self.table == tables::IMAGES {
                "image_labels"
            } else {
                "video_labels"
            };
            let hash_col = if self.table == tables::IMAGES {
                "image_hash"
            } else {
                "video_hash"
            };
            let user_col = if self.table == tables::IMAGES {
                "image_user_id"
            } else {
                "video_user_id"
            };
            format!("INNER JOIN {} l ON t.hash = l.{} AND t.user_id = l.{} AND l.label_id = ${}", label_table, hash_col, user_col, label_id_param)
        } else {
            String::new()
        };

        format!(
            "{} FROM {} t {} {} {}",
            select_clause,
            self.table,
            join_clause,
            label_join_clause,
            where_clause
        )
    }

    /// Build SELECT clause for listing thumbnails
    /// If lon_param and lat_param are provided, includes distance calculation
    pub fn build_select_query(&mut self, limit_param: usize, offset_param: usize, lon_param: Option<usize>, lat_param: Option<usize>, sort_by: Option<&str>, sort_order: Option<&str>) -> String {
        if !self.table_validated {
            return SAFE_FALLBACK_QUERY.to_string();
        }
        let body = self.build_select_body(lon_param, lat_param);
        let dir = if sort_order == Some("asc") { "ASC" } else { "DESC" };
        let order = if sort_by == Some("size") {
            format!("ORDER BY file_size_bytes {} NULLS LAST, hash {}", dir, dir)
        } else if sort_by == Some("quality") {
            format!("ORDER BY aesthetic_score {} NULLS LAST, hash {}", dir, dir)
        } else {
            format!("ORDER BY t.created_at {}, t.hash {}", dir, dir)
        };
        format!("{} {} LIMIT ${} OFFSET ${}", body, order, limit_param, offset_param)
    }

    /// Build COUNT query for total thumbnails
    pub fn build_count_query(&self, use_inner_join: bool) -> String {
        if !self.table_validated {
            return SAFE_FALLBACK_QUERY.to_string();
        }
        // Use appropriate starred table based on media type
        let starred_table = if self.table == tables::IMAGES {
            tables::STARRED_IMAGES
        } else {
            tables::STARRED_VIDEOS
        };

        let join_clause = if use_inner_join {
            if let Some(user_id_param) = self.user_id_param {
                format!("INNER JOIN {} s ON t.hash = s.hash AND s.user_id = ${}", starred_table, user_id_param)
            } else {
                format!("INNER JOIN {} s ON t.hash = s.hash", starred_table)
            }
        } else if let Some(user_id_param) = self.user_id_param {
            // Even when not filtering by starred_only, we need LEFT JOIN if user_id is set
            // to properly reserve the parameter position
            format!("LEFT JOIN {} s ON t.hash = s.hash AND s.user_id = ${}", starred_table, user_id_param)
        } else {
            String::new()
        };

        // Add label filtering join if needed
        let label_join_clause = if let Some(label_id_param) = self.label_id_param {
            let label_table = if self.table == tables::IMAGES {
                "image_labels"
            } else {
                "video_labels"
            };
            let hash_col = if self.table == tables::IMAGES {
                "image_hash"
            } else {
                "video_hash"
            };
            let user_col = if self.table == tables::IMAGES {
                "image_user_id"
            } else {
                "video_user_id"
            };
            format!("INNER JOIN {} l ON t.hash = l.{} AND t.user_id = l.{} AND l.label_id = ${}", label_table, hash_col, user_col, label_id_param)
        } else {
            String::new()
        };

        let where_clause = self.build_where_clause();

        format!(
            "SELECT COUNT(*) FROM {} t {} {} {}",
            self.table,
            join_clause,
            label_join_clause,
            where_clause
        )
    }

    /// Get current parameter count
    pub fn param_count(&self) -> usize {
        self.param_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_query_no_filters() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        let query = builder.build_select_query(1, 2, None, None, None, None);

        assert!(query.contains("LEFT JOIN starred_images"));
        assert!(query.contains("ORDER BY"));
        assert!(query.contains("LIMIT $1 OFFSET $2"));
        assert!(query.contains("WHERE t.deleted_at IS NULL"));
    }

    #[test]
    fn test_build_query_with_device_filter() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        builder.with_device_id();
        let query = builder.build_select_query(2, 3, None, None, None, None);

        assert!(query.contains("WHERE t.deleted_at IS NULL AND t.deviceid = $1"));
        assert!(query.contains("LIMIT $2 OFFSET $3"));
    }

    #[test]
    fn test_build_query_with_all_filters() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        builder.with_device_id();
        builder.with_starred_only();
        let query = builder.build_select_query(2, 3, None, None, None, None);

        assert!(query.contains("WHERE t.deleted_at IS NULL AND t.deviceid = $1 AND s.hash IS NOT NULL"));
        assert!(query.contains("LIMIT $2 OFFSET $3"));
    }

    #[test]
    fn test_build_count_query() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        builder.with_device_id();
        builder.with_has_thumbnail();
        let query = builder.build_count_query(false);

        assert!(query.contains("SELECT COUNT(*)"));
        assert!(query.contains("WHERE t.deleted_at IS NULL AND t.deviceid = $1 AND t.has_thumbnail = true"));
    }

    #[test]
    fn test_build_count_query_with_starred_join() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        builder.with_has_thumbnail();
        let query = builder.build_count_query(true);

        assert!(query.contains("INNER JOIN starred_images"));


        assert!(query.contains("WHERE t.deleted_at IS NULL AND t.has_thumbnail = true"));
    }

    #[test]
    fn test_user_id_and_access_control_filter() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        let user = builder.with_user_id();
        assert_eq!(user.position, 1);
        builder.with_user_id_filter();
        let query = builder.build_select_query(2, 3, None, None, None, None);
        assert!(query.contains("LEFT JOIN starred_images s ON t.hash = s.hash AND s.user_id = $1"));
        assert!(query.contains("t.user_id = $1"));
        assert!(query.contains("LIMIT $2 OFFSET $3"));
    }

    #[test]
    fn test_user_id_videos_uses_starred_videos() {
        let mut builder = MediaQueryBuilder::new(tables::VIDEOS);
        builder.with_user_id();
        builder.with_user_id_filter();
        let query = builder.build_select_query(2, 3, None, None, None, None);
        assert!(query.contains("LEFT JOIN starred_videos"));
        assert!(query.contains("t.user_id = $1"));
        assert!(query.contains("'video' as media_type"));
    }

    #[test]
    fn test_label_id_join_and_count() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        assert_eq!(builder.with_user_id().position, 1);
        assert_eq!(builder.with_label_id().position, 2);
        let select = builder.build_select_query(3, 4, None, None, None, None);
        assert!(select.contains("INNER JOIN image_labels l ON t.hash = l.image_hash AND t.user_id = l.image_user_id AND l.label_id = $2"));
        let count = builder.build_count_query(false);
        assert!(count.contains("INNER JOIN image_labels l"));
        assert!(count.contains("label_id = $2"));
    }

    #[test]
    fn test_video_label_tables() {
        let mut builder = MediaQueryBuilder::new(tables::VIDEOS);
        builder.with_user_id();
        builder.with_label_id();
        let select = builder.build_select_query(3, 4, None, None, None, None);
        assert!(select.contains("INNER JOIN video_labels l ON t.hash = l.video_hash AND t.user_id = l.video_user_id"));
    }

    #[test]
    fn test_date_and_no_exif_filters() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        builder.with_user_id();
        builder.with_start_date();
        builder.with_end_date();
        builder.with_no_exif();
        let query = builder.build_select_query(4, 5, None, None, None, None);
        assert!(query.contains("t.created_at >= $2"));
        assert!(query.contains("t.created_at < $3"));
        assert!(query.contains("t.exif IS NULL"));
        assert!(query.contains("LIMIT $4 OFFSET $5"));
    }

    #[test]
    fn test_geo_distance_parameters() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        builder.with_user_id();
        builder.with_starred_only();
        let query = builder.build_select_query(3, 4, Some(10), Some(11), None, None);
        assert!(query.contains("ST_Distance(t.location, ST_MakePoint($10, $11)::geography)"));
        assert!(query.contains("s.hash IS NOT NULL"));
    }

    #[test]
    fn test_is_table_type_helpers() {
        assert!(MediaQueryBuilder::new(tables::IMAGES).is_images_table());
        assert!(MediaQueryBuilder::new(tables::VIDEOS).is_videos_table());
    }

    #[test]
    fn test_invalid_table_falls_back_to_safe_query() {
        let mut builder = MediaQueryBuilder::new("images; DROP TABLE users");
        assert!(!builder.table_validated());
        let select = builder.build_select_query(1, 2, None, None, None, None);
        let count = builder.build_count_query(false);
        assert_eq!(select, "SELECT 1 WHERE false");
        assert_eq!(count, "SELECT 1 WHERE false");
        // No interpolated table anywhere in the emitted SQL.
        assert!(!select.contains("DROP"));
        assert!(!count.contains("DROP"));
    }
}