import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = path.dirname(fileURLToPath(import.meta.url));
const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8' };
http.createServer((req, res) => {
  const filename = path.resolve(root, '.' + (req.url === '/' ? '/index.html' : decodeURIComponent(req.url.split('?')[0])));
  if (!filename.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  fs.readFile(filename, (error, data) => { if (error) { res.writeHead(404).end('Not found'); return; } res.writeHead(200, { 'Content-Type': types[path.extname(filename)] || 'application/octet-stream', 'Cache-Control': 'no-store' }); res.end(data); });
}).listen(4317, '127.0.0.1', () => console.log('Local: http://127.0.0.1:4317'));
