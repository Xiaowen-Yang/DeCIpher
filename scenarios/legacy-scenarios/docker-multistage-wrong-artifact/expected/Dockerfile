FROM node:18-alpine AS builder
WORKDIR /src
COPY package.json index.js ./
RUN mkdir -p output && cp index.js output/app.js

FROM node:18-alpine AS runner
WORKDIR /app
# FIX: copy from the correct output directory /src/output/
COPY --from=builder /src/output/app.js ./app.js
CMD ["node", "app.js"]
