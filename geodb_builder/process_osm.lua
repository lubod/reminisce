-- Lua script for osm2pgsql flex backend
-- Filters only administrative boundaries to minimize database size

local tables = {}

-- Define the table schema matching our previous structure
tables.admin_boundaries = osm2pgsql.define_table({
    name = 'admin_boundaries',
    ids = { type = 'any', type_column = 'osm_type', id_column = 'osm_id' },
    columns = {
        { column = 'name', type = 'text' },
        { column = 'admin_level', type = 'integer' },
        { column = 'country_code', type = 'text' },
        { column = 'geometry', type = 'multipolygon', projection = 4326 },
        { column = 'tags', type = 'jsonb' }, -- Store other tags just in case for post-processing
    }
})

function osm2pgsql.process_node(object)
    -- We don't need nodes for admin boundaries
end

function osm2pgsql.process_relation(object)
    local tags = object.tags

    -- Filter: Must be boundary=administrative
    if tags.boundary ~= 'administrative' then
        return
    end

    -- Filter: Must have a name
    if not tags.name then
        return
    end

    -- Filter: Must have a valid admin_level
    local admin_level = tonumber(tags.admin_level)
    if not admin_level then
        return
    end

    -- We typically want levels 2 (Country), 4 (State), 6 (County), 8 (City), 10 (Sub-city/Village)
    -- Adjust this filter as needed.
    if admin_level ~= 2 and admin_level ~= 4 and admin_level ~= 6 and admin_level ~= 8 and admin_level ~= 10 then
        return
    end

    -- Extract Country Code
    local country_code = tags['ISO3166-1'] or tags['ISO3166-1:alpha2'] or tags['country_code']

    tables.admin_boundaries:insert({
        name = tags.name,
        admin_level = admin_level,
        country_code = country_code,
        geometry = object:as_multipolygon(),
        tags = tags
    })
end

-- We process relations for boundaries.
-- Sometimes boundaries are ways, but high-level ones are almost exclusively relations.
function osm2pgsql.process_way(object)
    local tags = object.tags

    if tags.boundary == 'administrative' and tags.name and tags.admin_level then
         -- Verify way is closed before attempting polygon creation
         if not object.is_closed then
             return
         end

         local admin_level = tonumber(tags.admin_level)
         if admin_level and (admin_level == 2 or admin_level == 4 or admin_level == 6 or admin_level == 8 or admin_level == 10) then
             local country_code = tags['ISO3166-1'] or tags['ISO3166-1:alpha2'] or tags['country_code']
             
             tables.admin_boundaries:insert({
                name = tags.name,
                admin_level = admin_level,
                country_code = country_code,
                geometry = object:as_polygon(),
                tags = tags
            })
         end
    end
end
