FROM node:18-alpine

WORKDIR /app

# Inline HTTP server that listens on port 3000
RUN printf 'const http = require("http");\nconst server = http.createServer((req, res) => {\n  res.writeHead(200);\n  res.end("OK");\n});\nserver.listen(3000, () => console.log("Server listening on port 3000"));\n' > server.js

EXPOSE 3000

# BUG: healthcheck probes port 8080 but the app listens on 3000
HEALTHCHECK --interval=5s --timeout=3s --retries=3 \
  CMD wget -qO- http://localhost:8080/ || exit 1

CMD ["node", "server.js"]
