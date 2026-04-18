'use strict';
const http = require('http');

const dbUrl = process.env.DATABASE_URL;

if (!dbUrl) {
  console.error('FATAL: DATABASE_URL environment variable is required but not set');
  console.error('Set it via -e DATABASE_URL=<url> or add ENV in the Dockerfile');
  process.exit(1);
}

const port = parseInt(process.env.PORT || '3000', 10);

http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ status: 'ok', db: dbUrl }));
}).listen(port, () => {
  console.log(`Server running on port ${port}, db=${dbUrl}`);
});
