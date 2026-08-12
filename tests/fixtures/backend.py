from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length).decode("utf-8")
        response = "\n".join(
            [
                f"method={self.command}",
                f"path={self.path}",
                f"body={body}",
                f"x-added={self.headers.get('x-added', '')}",
                f"x-remove-me={self.headers.get('x-remove-me', '')}",
                f"x-request-id={self.headers.get('x-request-id', '')}",
                f"x-lungyam-route={self.headers.get('x-lungyam-route', '')}",
            ]
        ).encode("utf-8")

        self.send_response(200)
        self.send_header("content-type", "text/plain")
        self.send_header("content-length", str(len(response)))
        self.send_header("x-backend", "fixture")
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, _format, *_args):
        return


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", 3001), Handler).serve_forever()
