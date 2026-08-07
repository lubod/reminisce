s = open("src/services/text_search.rs").read()

# Videos have no `location` column, so location filtering must only apply to
# the images arm. Old block (appears twice: hybrid where + text anchor):
old_block = """        if let (Some(lon), Some(lat)) = (lon_param, lat_param) {
            conds.push(format!("{}.location IS NOT NULL", alias));
            conds.push(format!(
                "ST_DWithin({}.location, ST_MakePoint(${}, ${})::geography, {})",
                alias, lon, lat, radius_meters
            ));
        }"""
new_block = """        if alias == "i" {
            if let (Some(lon), Some(lat)) = (lon_param, lat_param) {
                conds.push(format!("{}.location IS NOT NULL", alias));
                conds.push(format!(
                    "ST_DWithin({}.location, ST_MakePoint(${}, ${})::geography, {})",
                    alias, lon, lat, radius_meters
                ));
            }
        }"""
count = s.count(old_block)
assert count == 2, f"expected 2 location blocks, found {count}"
s = s.replace(old_block, new_block)

# Video distance expression must not touch v.location.
old_dist = 'let vid_distance = build_distance_expr(lon_param, lat_param, "v");'
new_dist = 'let vid_distance = build_distance_expr(None, None, "v");'
count2 = s.count(old_dist)
print("vid_distance occurrences:", count2)
s = s.replace(old_dist, new_dist)

open("src/services/text_search.rs", "w").write(s)
print("text_search.rs fixed")
