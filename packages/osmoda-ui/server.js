// osModa UI — lightweight server + reverse proxy to OpenClaw gateway
const http = require('http');
const fs = require('fs');
const net = require('net');
const path = require('path');

const PORT = parseInt(process.env.PORT || '18789');
const OPENCLAW_PORT = parseInt(process.env.OPENCLAW_PORT || '18790');
const HTML = fs.readFileSync(path.join(__dirname, 'index.html'), 'utf-8');

const server = http.createServer(function(req, res) {
  // Serve custom chat UI
  if (req.url === '/' && (req.method === 'GET' || req.method === 'HEAD')) {
    res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    if (req.method === 'GET') res.end(HTML);
    else res.end();
    return;
  }

  // Health check
  if (req.url === '/health') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end('{"ok":true}');
    return;
  }

  // Proxy everything else to OpenClaw gateway
  var proxyHeaders = Object.assign({}, req.headers);
  proxyHeaders.host = '127.0.0.1:' + OPENCLAW_PORT;

  var proxyReq = http.request({
    hostname: '127.0.0.1',
    port: OPENCLAW_PORT,
    path: req.url,
    method: req.method,
    headers: proxyHeaders,
    timeout: 120000,
  }, function(proxyRes) {
    res.writeHead(proxyRes.statusCode, proxyRes.headers);
    proxyRes.pipe(res);
  });

  proxyReq.on('timeout', function() { proxyReq.destroy(); });
  proxyReq.on('error', function() {
    if (!res.headersSent) {
      res.writeHead(502, { 'Content-Type': 'application/json' });
      res.end('{"error":"gateway unavailable"}');
    }
  });

  req.pipe(proxyReq);
});

// WebSocket proxy: forward all upgrade requests to OpenClaw
server.on('upgrade', function(req, clientSocket, head) {
  var proxySocket = net.connect(OPENCLAW_PORT, '127.0.0.1', function() {
    // Forward the raw HTTP upgrade request
    var reqLine = req.method + ' ' + req.url + ' HTTP/' + req.httpVersion + '\r\n';
    var headers = Object.keys(req.headers).map(function(k) {
      return k + ': ' + req.headers[k];
    }).join('\r\n');
    proxySocket.write(reqLine + headers + '\r\n\r\n');
    if (head && head.length) proxySocket.write(head);

    // Bidirectional pipe
    clientSocket.pipe(proxySocket);
    proxySocket.pipe(clientSocket);
  });

  proxySocket.on('error', function() { clientSocket.destroy(); });
  clientSocket.on('error', function() { proxySocket.destroy(); });
});

server.listen(PORT, '127.0.0.1', function() {
  console.log('osModa UI: http://127.0.0.1:' + PORT);
  console.log('Proxying to OpenClaw: http://127.0.0.1:' + OPENCLAW_PORT);
});
