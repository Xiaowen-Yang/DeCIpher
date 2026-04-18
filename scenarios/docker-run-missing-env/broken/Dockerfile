FROM node:18-alpine
WORKDIR /app
COPY package.json server.js ./
EXPOSE 3000
# BUG: DATABASE_URL is required by the app but no default is set here
ENV DATABASE_URL=sqlite:///app/data.db
CMD ["node", "server.js"]
