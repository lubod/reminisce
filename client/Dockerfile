# Build stage
FROM node:20-alpine AS build

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
FROM nginx:alpine

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
