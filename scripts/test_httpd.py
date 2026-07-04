#!/usr/bin/env python3
"""Tiny host-side HTTP server for the LightOS TCP boot test.

Listens on 127.0.0.1:18080. QEMU SLIRP maps this to 10.0.2.2 as seen
from the guest, so the guest's `httpget` reaches it without any real
network egress — the test stays hermetic and CI-safe. Every GET returns
a fixed body containing the marker `LightOS-TCP-OK`.
"""
import http.server
import socketserver

PORT = 18080
BODY = b"LightOS-TCP-OK: hello from the host over real TCP\n"


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(BODY)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(BODY)

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    socketserver.TCPServer.allow_reuse_address = True
    with socketserver.TCPServer(("127.0.0.1", PORT), Handler) as srv:
        srv.serve_forever()
