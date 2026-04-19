FROM node:18-alpine
WORKDIR /app
COPY server.js .
EXPOSE 3000
# FIX: healthcheck now probes the correct port (3000)
HEALTHCHECK --interval=5s --timeout=3s --retries=2 \
  CMD wget -qO- http://localhost:3000/healthz || exit 1
CMD ["node", "server.js"]
