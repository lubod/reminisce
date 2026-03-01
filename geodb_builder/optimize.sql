
-- Create indexes
CREATE INDEX IF NOT EXISTS idx_admin_boundaries_geometry ON admin_boundaries USING GIST(geometry);
CREATE INDEX IF NOT EXISTS idx_admin_boundaries_admin_level ON admin_boundaries(admin_level);
CREATE INDEX IF NOT EXISTS idx_admin_boundaries_name ON admin_boundaries(name);

-- Optimization: Simplify geometries to reduce size
-- Unified strategy: ~110m tolerance (0.001 degrees) for ALL levels.
-- This strikes the best balance between size and accuracy for "City/Country" geotagging.
-- It is crucial to apply this to Level 2 (Countries) which are otherwise huge.
UPDATE admin_boundaries
SET geometry = ST_SimplifyPreserveTopology(geometry, 0.001);

-- Vacuum to reclaim space after updates
VACUUM FULL admin_boundaries;

-- Analyze for query planner
ANALYZE admin_boundaries;

-- Verification
SELECT admin_level, count(*) as count, pg_size_pretty(pg_total_relation_size('admin_boundaries')) as size
FROM admin_boundaries
GROUP BY admin_level
ORDER BY admin_level;
