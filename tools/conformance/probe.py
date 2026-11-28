"""Probe a gateway and a tariff MCP service through their public network contracts."""
import argparse
import base64
import datetime
import hmac
import http.client
import json
import os
import queue
import re
import secrets
import sys
import threading
import time
import tomllib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import quote, urlsplit

PING = "de.ems.test.ping"
ROOT = Path(__file__).resolve().parents[2]
CONTRACTS = tomllib.loads((ROOT / "platform.toml").read_text())["contracts"]
PROTOCOL = CONTRACTS["mcp_protocol"]
LIMIT = 4 * 1024 * 1024


class Failure(Exception):
    """A contract check failed. Messages may be included in release logs."""


def require(condition, message):
    if not condition:
        raise Failure(message)


def sse_result(reply, rpc_id):
    """Read one matching RPC reply. A peer may keep the event stream open."""
    budget = LIMIT
    deadline = time.monotonic() + 12
    lines = []
    while budget > 0 and time.monotonic() < deadline:
        line = reply.readline(budget + 1)
        if not line:
            break
        budget -= len(line)
        require(budget >= 0, "MCP event stream exceeds the size limit")
        line = line.rstrip(b"\r\n")
        if line.startswith(b"data:"):
            lines.append(line[5:].removeprefix(b" "))
        if not line:
            data = b"\n".join(lines)
            lines.clear()
            if data.strip() and json.loads(data).get("id") == rpc_id:
                return data
    raise Failure("MCP stream ended or exceeded the limit before the RPC result")


def request(base, path, *, token=None, method="GET", body=None, headers=None, expected=200, rpc_id=None):
    url = urlsplit(base)
    require(url.scheme in ("http", "https") and url.hostname and not url.username
            and not url.password and not url.query and not url.fragment, "Invalid service URL")
    transport = http.client.HTTPSConnection if url.scheme == "https" else http.client.HTTPConnection
    conn = transport(url.hostname, url.port, timeout=12)
    request_headers = dict(headers or {})
    if token:
        request_headers["Authorization"] = "Bearer " + token
    if body is not None:
        request_headers["Content-Type"] = "application/json"
    try:
        conn.request(method, url.path.rstrip("/") + path,
                     None if body is None else json.dumps(body).encode(), request_headers)
        reply = conn.getresponse()
        require(reply.status == expected, f"{method} {path}: expected {expected}, got {reply.status}")
        if rpc_id is not None and reply.headers.get_content_type() == "text/event-stream":
            return reply.headers, sse_result(reply, rpc_id)
        data = reply.read(LIMIT + 1)
        require(len(data) <= LIMIT, "Response exceeds probe limit")
        return reply.headers, data
    finally:
        conn.close()


def receiver(path, deliveries):
    class Callback(BaseHTTPRequestHandler):
        def setup(self):
            super().setup()
            self.connection.settimeout(5)

        def do_POST(self):
            size = self.headers.get("Content-Length", "")
            if self.path != path or not size.isdecimal() or not 0 < int(size) <= 65536:
                self.send_error(400)
                return
            try:
                deliveries.put_nowait((self.headers, self.rfile.read(int(size))))
            except queue.Full:
                self.send_error(409)
                return
            self.send_response(204)
            self.end_headers()

        def log_message(self, *args):
            pass  # Callback URLs are private.

    return Callback


def verify_delivery(delivery, secret, site):
    headers, body = delivery
    require(headers.get_content_type() == "application/cloudevents+json", "Wrong event media type")
    message_id = headers.get("webhook-id", "")
    stamp = headers.get("webhook-timestamp", "")
    require(stamp.isascii() and stamp.isdecimal() and abs(int(time.time()) - int(stamp)) <= 300,
            "Invalid delivery timestamp")
    require(secret.startswith("whsec_"), "Invalid subscription secret")
    key = base64.b64decode(secret[6:], validate=True)
    require(24 <= len(key) <= 64 and base64.b64encode(key).decode("ascii") == secret[6:],
            "Invalid subscription key length or encoding")
    signed = message_id.encode() + b"." + stamp.encode() + b"." + body
    signature = b"v1," + base64.b64encode(hmac.digest(key, signed, "sha256"))
    require(any(hmac.compare_digest(signature, s.encode()) for s in
                headers.get("webhook-signature", "").split()), "Invalid webhook signature")
    event = json.loads(body)
    require(message_id and event["id"] == message_id and event["specversion"] == "1.0"
            and event["type"] == PING and event["data"]["test"] is True
            and event.get("dataschema") == f"https://hupe1980.github.io/ems/contracts/events/v1/{PING}.json#/properties/data"
            and event["source"] == f"ems://sites/{site}"
            and event.get("datacontenttype", "application/json") == "application/json",
            "Incorrect ping envelope, site or payload")
    instant = datetime.datetime.fromisoformat(event["time"].replace("Z", "+00:00"))
    require(instant.tzinfo and abs(instant.timestamp() - int(stamp)) <= 300, "Invalid ping time")


def gateway(args, token):
    contract = json.loads(request(args.gateway, "/openapi.json")[1])
    published = {Path(path).stem for path in CONTRACTS["event_schemas"]}
    require(PING in published and published <= contract["webhooks"].keys()
            and "/v1/status" in contract["paths"], "Incomplete gateway OpenAPI contract")
    request(args.gateway, "/v1/status", expected=401)
    request(args.gateway, "/v1/status", token=secrets.token_urlsafe(32), expected=401)
    site = json.loads(request(args.gateway, "/v1/status", token=token)[1])["site"]
    require(isinstance(site, str) and site, "Missing household identity")
    path = "/" + secrets.token_urlsafe(24)
    deliveries = queue.Queue(maxsize=1)
    callback = urlsplit(args.callback)
    require(callback.scheme == "http" and callback.hostname and callback.path in ("", "/")
            and not callback.query and not callback.fragment and not callback.username
            and not callback.password, "Use a private HTTP callback origin")
    with ThreadingHTTPServer((args.bind, args.port), receiver(path, deliveries)) as server:
        threading.Thread(target=server.serve_forever, daemon=True).start()
        prefix = "/v1/subscriptions"
        try:
            created = json.loads(request(args.gateway, prefix, token=token, method="POST", expected=201,
                                         body={"name": "EMS conformance", "types": [PING],
                                               "url": args.callback.rstrip("/") + path})[1])
            sub_id = created["id"]
            require(isinstance(sub_id, str) and re.fullmatch(r"[A-Za-z0-9_-]+", sub_id), "Missing subscription identity")
            url = prefix + "/" + quote(sub_id, safe="")
            try:
                secret = created["secret"]
                listing = json.loads(request(args.gateway, prefix, token=token)[1])
                require(any(row["id"] == sub_id for row in listing) and
                        all("secret" not in row for row in listing) and secret not in json.dumps(listing),
                        "Invalid subscription listing or disclosed secret")
                result = json.loads(request(args.gateway, url + "/test", token=token, method="POST")[1])
                require(result["delivered"] is True and result["status"] == 204, "Callback delivery failed")
                verify_delivery(deliveries.get(timeout=5), secret, site)
            finally:
                request(args.gateway, url + "/revoke", token=token, method="POST", expected=204)
            request(args.gateway, url + "/test", token=token, method="POST", expected=404)
        finally:
            server.shutdown()


def mcp(base, token):
    metadata = {"io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientInfo": {"name": "ems-conformance", "version": "1"},
                "io.modelcontextprotocol/clientCapabilities": {}}

    def rpc(number, method, params=None, *, credential=token, expected=200):
        params = dict(params or {}, _meta=metadata)
        body = {"jsonrpc": "2.0", "id": number, "method": method, "params": params}
        headers = {"Accept": "application/json, text/event-stream", "MCP-Protocol-Version": PROTOCOL,
                   "Mcp-Method": method}
        if "name" in params:
            headers["Mcp-Name"] = params["name"]
        response, payload = request(base, "/mcp", method="POST", token=credential, headers=headers,
                                    body=body, expected=expected, rpc_id=number if expected == 200 else None)
        if expected != 200:
            return
        require(response.get_content_type() in ("application/json", "text/event-stream"), "Unknown MCP media type")
        message = json.loads(payload)
        require(message.get("jsonrpc") == "2.0" and message.get("id") == number
                and "result" in message, f"Missing MCP result for {method}")
        return message["result"]

    rpc(1, "server/discover", credential=None, expected=401)
    rpc(1, "server/discover", credential=secrets.token_urlsafe(32), expected=401)
    discovery = rpc(1, "server/discover")
    require(PROTOCOL in discovery["supportedVersions"] and "tools" in discovery["capabilities"],
            "MCP version or tools capability missing")
    tools = rpc(2, "tools/list")["tools"]
    require(any(t["name"] == "get_coverage" for t in tools), "Missing get_coverage tool")
    result = rpc(3, "tools/call", {"name": "get_coverage", "arguments": {"slots": 4}})
    require(not result.get("isError") and result["content"][0]["type"] == "text", "Coverage call failed")
    coverage = json.loads(result["content"][0]["text"])
    require(coverage["asked_for"] == 4 and type(coverage["cached_slots"]) is int
            and coverage["cached_slots"] >= 0 and type(coverage["coverage"]) in (int, float)
            and 0 <= coverage["coverage"] <= 1 and "contiguous_until" in coverage
            and (coverage["cached_slots"] or (coverage["coverage"] == 0 and coverage["contiguous_until"] is None)),
            "Invalid four-slot tariff coverage")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gateway", required=True, help="gateway base URL (HEMS_TOKEN)")
    parser.add_argument("--tariff", required=True, help="tariff base URL (MCP_TOKEN)")
    parser.add_argument("--callback", required=True, help="private HTTP origin reachable from the gateway")
    parser.add_argument("--bind", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=8085, help="local callback listener port")
    parser.add_argument("--wait", type=int, default=0, help="seconds to wait for service sockets at startup")
    args = parser.parse_args()
    try:
        for var in ("HEMS_TOKEN", "MCP_TOKEN"):
            require(os.environ.get(var), f"Set {var} before running the probe")
        if args.wait:
            deadline = time.monotonic() + args.wait
            for base in (args.gateway, args.tariff):
                while True:
                    try:
                        request(base, "/livez")
                        break
                    except (Failure, OSError, http.client.HTTPException):
                        if time.monotonic() >= deadline:
                            raise Failure("Services did not start before the readiness deadline") from None
                        time.sleep(0.25)
        mcp(args.tariff, os.environ["MCP_TOKEN"])
        gateway(args, os.environ["HEMS_TOKEN"])
    except Failure as err:
        sys.exit(str(err))
    except (OSError, http.client.HTTPException, ValueError, LookupError, TypeError, queue.Empty) as err:
        sys.exit(f"Invalid contract response or unavailable endpoint ({type(err).__name__})")
    print(json.dumps({"rest": "passed", "cloudevents": "passed", "webhooks": "passed", "mcp": "passed"}))


if __name__ == "__main__":
    main()
