#!/usr/bin/env python3
"""Minimal OpenAI-compatible proxy for trying AI Town with Gemini or NVIDIA NIM.

    GEMINI_API_KEY=... python3 examples/llm-proxy.py gemini
    NVIDIA_API_KEY=... python3 examples/llm-proxy.py nvidia

It serves POST /v1/chat/completions on 127.0.0.1:4000, replaces the `town-model`
alias with the upstream model, and forwards the request with the provider key.
The provider key stays in this process; the AI Town backend only needs

    LLM_BASE_URL=http://127.0.0.1:4000/v1 LLM_MODEL=town-model

Meant for local testing: no retries, streaming, or rate limiting. Logs show
status and timing only, never keys or message bodies. Standard library only.
"""

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PROVIDERS = {
    "gemini": {
        "base": "https://generativelanguage.googleapis.com/v1beta/openai",
        "key": "GEMINI_API_KEY",
        "model": "gemini-3.1-flash-lite",
    },
    "nvidia": {
        "base": "https://integrate.api.nvidia.com/v1",
        "key": "NVIDIA_API_KEY",
        "model": "google/diffusiongemma-26b-a4b-it",
    },
}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("provider", choices=sorted(PROVIDERS))
    ap.add_argument("--model", help="upstream model (default per provider)")
    ap.add_argument("--alias", default="town-model", help="model name AI Town sends (default town-model)")
    ap.add_argument("--port", type=int, default=4000)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--upstream", help="override the upstream API root")
    ap.add_argument("--timeout", type=float, default=19.0, help="upstream timeout in seconds (AI Town waits 20)")
    ap.add_argument("--token", default=os.environ.get("PROXY_TOKEN"),
                    help="require this bearer token from clients (LLM_API_KEY); default PROXY_TOKEN or none")
    args = ap.parse_args()

    p = PROVIDERS[args.provider]
    key = os.environ.get(p["key"], "")
    if not key:
        sys.exit(f"{p['key']} is not set")
    url = (args.upstream or p["base"]).rstrip("/") + "/chat/completions"
    model = args.model or p["model"]

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def reply(self, status, body):
            data = json.dumps(body).encode() if not isinstance(body, bytes) else body
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def authorized(self):
            if args.token and self.headers.get("Authorization") != f"Bearer {args.token}":
                self.reply(401, {"error": {"message": "invalid proxy token"}})
                return False
            return True

        def do_GET(self):
            if self.path.rstrip("/") == "/v1/models" and self.authorized():
                self.reply(200, {"object": "list", "data": [{"id": args.alias, "object": "model"}]})
            elif self.path.rstrip("/") != "/v1/models":
                self.reply(404, {"error": {"message": "not found"}})

        def do_POST(self):
            if self.path.rstrip("/") != "/v1/chat/completions":
                return self.reply(404, {"error": {"message": "not found"}})
            if not self.authorized():
                return
            try:
                req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
            except (ValueError, json.JSONDecodeError):
                return self.reply(400, {"error": {"message": "invalid JSON"}})
            if req.get("model") != args.alias:
                return self.reply(404, {"error": {"message": f"unknown model; use {args.alias}"}})
            req["model"] = model
            req["stream"] = False
            up = urllib.request.Request(url, data=json.dumps(req).encode(), method="POST", headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {key}",
            })
            started = time.monotonic()
            try:
                with urllib.request.urlopen(up, timeout=args.timeout) as r:
                    status, body = r.status, r.read()
            except urllib.error.HTTPError as e:
                status, body = e.code, e.read()
            except Exception as e:  # timeout, DNS, TLS, connection reset
                status, body = 504, json.dumps({"error": {"message": f"upstream: {type(e).__name__}"}}).encode()
            sys.stderr.write(f"{args.provider} {model} -> HTTP {status} in {time.monotonic() - started:.2f}s\n")
            self.reply(status, body)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    sys.stderr.write(f"proxy: http://{args.host}:{args.port}/v1  {args.alias} -> {args.provider} {model}\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
