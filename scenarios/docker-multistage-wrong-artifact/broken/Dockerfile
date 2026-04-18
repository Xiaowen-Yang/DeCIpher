FROM node:18-alpine AS builder
WORKDIR /src
COPY package.json index.js ./
RUN mkdir -p output && cp index.js output/app.js

FROM node:18-alpine AS runner
WORKDIR /app
# BUG: builder wrote to /src/output/ but we copy from /src/dist/ — path does not exist
COPY --from=builder /src/dist/app.js ./app.js
CMD ["node", "app.js"]
