'use strict';
const http = require('http');

const PORT = 3000;

http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/plain' });
  res.end(req.url === '/healthz' ? 'OK\n' : 'Hello from app\n');
}).listen(PORT, () => {
  console.log(`Server listening on port ${PORT}`);
});
