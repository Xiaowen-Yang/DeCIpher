FROM node:18-alpine
WORKDIR /app
COPY package.json server.js ./
EXPOSE 3000
# BUG: DATABASE_URL is required by the app but no default is set here
CMD ["node", "server.js"]
