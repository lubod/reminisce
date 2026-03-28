//! Query builder utilities for constructing SQL queries dynamically
//!
//! This module provides utilities to safely build SQL queries with dynamic
//! WHERE conditions and parameters, avoiding string concatenation errors.

use crate::constants::tables;

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
}

impl MediaQueryBuilder {
    /// Create a new query builder for the given table
    pub fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            conditions: vec!["t.deleted_at IS NULL".to_string()],
            param_count: 0,
            user_id_param: None,
            label_id_param: None,
        }
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

    /// Add media type filter condition
    pub fn with_media_type(&mut self) -> QueryParam {
        self.param_count += 1;
        self.conditions.push(format!("t.type = ${}", self.param_count));
        QueryParam { position: self.param_count }
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
        let has_location = lon_param.is_some() && lat_param.is_some();

        let select_clause = if self.table == tables::IMAGES {
            if has_location {
                let lon_p = lon_param.unwrap();
                let lat_p = lat_param.unwrap();
                format!(
                    "SELECT t.hash, t.name, t.created_at, t.place, t.deviceid, \
                     CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred, \
                     ST_Distance(t.location, ST_MakePoint(${}, ${})::geography) / 1000.0 as distance_km, \
                     'image' as media_type, t.file_size_bytes::bigint as file_size_bytes",
                    lon_p, lat_p
                )
            } else {
                "SELECT t.hash, t.name, t.created_at, t.place, t.deviceid, \
                 CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred, \
                 NULL::double precision as distance_km, \
                 'image' as media_type, t.file_size_bytes::bigint as file_size_bytes".to_string()
            }
        } else {
            "SELECT t.hash, t.name, t.created_at, NULL as place, t.deviceid, \
             CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred, \
             NULL::double precision as distance_km, \
             'video' as media_type, t.file_size_bytes as file_size_bytes".to_string()
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
            format!("INNER JOIN {} l ON t.hash = l.{} AND l.label_id = ${}", label_table, hash_col, label_id_param)
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
    pub fn build_select_query(&mut self, limit_param: usize, offset_param: usize, lon_param: Option<usize>, lat_param: Option<usize>) -> String {
        let body = self.build_select_body(lon_param, lat_param);
        format!(
            "{} ORDER BY t.created_at DESC, t.hash DESC LIMIT ${} OFFSET ${}",
            body,
            limit_param,
            offset_param
        )
    }

    /// Build COUNT query for total thumbnails
    pub fn build_count_query(&self, use_inner_join: bool) -> String {
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
            format!("INNER JOIN {} l ON t.hash = l.{} AND l.label_id = ${}", label_table, hash_col, label_id_param)
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
        let query = builder.build_select_query(1, 2, None, None);

        assert!(query.contains("LEFT JOIN starred_images"));
        assert!(query.contains("ORDER BY"));
        assert!(query.contains("LIMIT $1 OFFSET $2"));
        assert!(query.contains("WHERE t.deleted_at IS NULL"));
    }

    #[test]
    fn test_build_query_with_device_filter() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        builder.with_device_id();
        let query = builder.build_select_query(2, 3, None, None);

        assert!(query.contains("WHERE t.deleted_at IS NULL AND t.deviceid = $1"));
        assert!(query.contains("LIMIT $2 OFFSET $3"));
    }

    #[test]
    fn test_build_query_with_all_filters() {
        let mut builder = MediaQueryBuilder::new(tables::IMAGES);
        builder.with_device_id();
        builder.with_media_type();
        builder.with_starred_only();
        let query = builder.build_select_query(3, 4, None, None);

        assert!(query.contains("WHERE t.deleted_at IS NULL AND t.deviceid = $1 AND t.type = $2 AND s.hash IS NOT NULL"));
        assert!(query.contains("LIMIT $3 OFFSET $4"));
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
}
