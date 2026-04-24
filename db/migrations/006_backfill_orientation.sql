-- Backfill orientation column from EXIF JSON for existing images.
-- The exif column stores kamadak-exif display_value() strings for the Orientation tag.
-- Map those strings to their EXIF numeric equivalents (1–8).
UPDATE images SET orientation = CASE exif::json->>'Orientation'
    WHEN 'row 0 at top and column 0 at left'     THEN 1
    WHEN 'row 0 at top and column 0 at right'    THEN 2
    WHEN 'row 0 at bottom and column 0 at right' THEN 3
    WHEN 'row 0 at bottom and column 0 at left'  THEN 4
    WHEN 'row 0 at left and column 0 at top'     THEN 5
    WHEN 'row 0 at right and column 0 at top'    THEN 6
    WHEN 'row 0 at right and column 0 at bottom' THEN 7
    WHEN 'row 0 at left and column 0 at bottom'  THEN 8
    ELSE NULL
END
WHERE exif IS NOT NULL AND orientation IS NULL;
