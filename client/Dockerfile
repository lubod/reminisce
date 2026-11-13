# Build stage
FROM node:24-alpine@sha256:116b800c3754b440ee0aa3ed2f20f5bf3d99a554bfd9adeea09ea8017f6e85f5 AS build

WORKDIR /app

COPY package*.json ./
# Use cache mount for npm registry
RUN --mount=type=cache,target=/root/.npm \
    npm ci --prefer-offline

COPY . .
# Use cache mount for vite/build artifacts
RUN --mount=type=cache,target=/app/node_modules/.cache \
    npm run build

# Production stage
FROM nginx:alpine@sha256:4a73073bd557c65b759505da037898b61f1be6cbcc3c2c3aeac22d2a470c1752

# Install openssl for certificate generation if needed
RUN apk add --no-cache openssl

# Create SSL directory
RUN mkdir -p /etc/nginx/ssl

COPY --from=build /app/dist /usr/share/nginx/html
COPY nginx.conf /etc/nginx/conf.d/default.conf

# Copy SSL certificate generation script
COPY generate-ssl-certs.sh /docker-entrypoint.d/00-generate-certs.sh
RUN chmod +x /docker-entrypoint.d/00-generate-certs.sh

EXPOSE 80 443

CMD ["nginx", "-g", "daemon off;"]
