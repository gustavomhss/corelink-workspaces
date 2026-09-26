#!/usr/bin/env python3
from __future__ import annotations

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import re
import threading


ROUTE = re.compile(r"^/v1/(?:ac|cas)/issue-613-test/[a-f0-9]{64}$")
TOKEN = "Bearer issue-613-local-only"
records: dict[str, bytes] = {}
records_lock = threading.Lock()
request_log_lock = threading.Lock()
request_count = 0


class Handler(BaseHTTPRequestHandler):
    server_version = "issue-613-loopback/1"

    def _record_request(self, status: int) -> None:
        global request_count
        with request_log_lock:
            if request_count >= 512:
                return
            with open(self.server.request_log, "a", encoding="ascii") as stream:
                stream.write(f"{self.command} {self.path} {status}\n")
            request_count += 1

    def _respond(self, status: int, body: bytes = b"") -> None:
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)
        self._record_request(status)

    def _valid_request(self) -> bool:
        return (
            self.client_address[0] == "127.0.0.1"
            and ROUTE.fullmatch(self.path) is not None
            and self.headers.get("Authorization") == TOKEN
        )

    def do_GET(self) -> None:
        if not self._valid_request():
            self._respond(403)
            return
        with records_lock:
            value = records.get(self.path)
        if value is None:
            self._respond(404)
        else:
            self._respond(200, value)

    def do_PUT(self) -> None:
        if not self._valid_request():
            self._respond(403)
            return
        size = int(self.headers.get("Content-Length", "0"))
        if size < 0 or size > 5 * 1024 * 1024:
            self._respond(413)
            return
        value = self.rfile.read(size)
        with records_lock:
            records[self.path] = value
        self._respond(201)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port-file", required=True)
    parser.add_argument("--request-log", required=True)
    args = parser.parse_args()

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.request_log = str(Path(args.request_log))
    Path(args.port_file).write_text(str(server.server_address[1]), encoding="ascii")
    server.serve_forever(poll_interval=0.1)


if __name__ == "__main__":
    main()
