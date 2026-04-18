FROM node:18-alpine
WORKDIR /app
COPY package.json server.js ./
EXPOSE 3000
# FIX: provide a safe default so the container starts without manual -e flag
ENV DATABASE_URL=sqlite:///app/data.db
CMD ["node", "server.js"]
