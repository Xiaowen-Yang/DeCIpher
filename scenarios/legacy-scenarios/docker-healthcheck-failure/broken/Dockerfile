FROM node:18-alpine
WORKDIR /app
COPY server.js .
EXPOSE 3000
# BUG: healthcheck probes port 8080 but server listens on 3000
HEALTHCHECK --interval=5s --timeout=3s --retries=2 \
  CMD wget -qO- http://localhost:8080/healthz || exit 1
CMD ["node", "server.js"]
