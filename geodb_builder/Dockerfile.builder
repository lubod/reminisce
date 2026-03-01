FROM debian:bookworm-slim

# Install PostgreSQL 16, PostGIS, osm2pgsql (1.8.0+), and tools
# We add the official PG repo to ensure we get version 16 specifically
RUN apt-get update && apt-get install -y \
    wget \
    gnupg2 \
    lsb-release \
    ca-certificates \
    && echo "deb http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list \
    && wget --quiet -O - https://www.postgresql.org/media/keys/ACCC4CF8.asc | apt-key add - \
    && apt-get update \
    && apt-get install -y \
        postgresql-16 \
        postgresql-16-postgis-3 \
        osm2pgsql \
        osmium-tool \
    && rm -rf /var/lib/apt/lists/*

# Add postgres binaries to path
ENV PATH="/usr/lib/postgresql/16/bin:${PATH}"

# Environment variables for postgres
ENV POSTGRES_USER=postgres
ENV POSTGRES_PASSWORD=postgres
ENV POSTGRES_DB=geotagging_db

WORKDIR /app

# Copy processing scripts
COPY process_osm.lua /app/
COPY optimize.sql /app/

# Entrypoint script to handle the build process
COPY entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/entrypoint.sh

ENTRYPOINT ["entrypoint.sh"]
ENV POSTGRES_USER=postgres
ENV POSTGRES_PASSWORD=postgres
ENV POSTGRES_DB=geotagging_db

WORKDIR /app

# Copy processing scripts
COPY process_osm.lua /app/
COPY optimize.sql /app/

# Entrypoint script to handle the build process
COPY entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/entrypoint.sh

ENTRYPOINT ["entrypoint.sh"]