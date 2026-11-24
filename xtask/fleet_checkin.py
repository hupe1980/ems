"""Send simulated gateway check-ins over fleetd's HTTP interface."""

import argparse
import ipaddress
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request


class DirectRequest(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("url", help="Fleet origin, e.g. http://127.0.0.1:8080")
    parser.add_argument("--count", type=int, default=1, help="Check-ins, 30 seconds apart")
    parser.add_argument("--stalled-feeder", action="store_true", help="Freeze feeder evidence three minutes in the past")
    args = parser.parse_args()
    url = urllib.parse.urlsplit(args.url)
    try:
        loopback = ipaddress.ip_address(url.hostname or "").is_loopback
    except ValueError:
        loopback = False
    if not url.hostname or url.username is not None or url.path not in ("", "/") or url.query or url.fragment or not (
        url.scheme == "https" or (url.scheme == "http" and loopback)
    ):
        parser.error("Use an HTTPS origin or an HTTP loopback IP address.")
    token = os.environ.get("FLEET_SITE_TOKEN")
    if not token or args.count < 1:
        parser.error("Set FLEET_SITE_TOKEN and use a positive count.")
    opener = urllib.request.build_opener(DirectRequest)
    frozen = int(time.time()) - 180
    for index in range(args.count):
        if index:
            time.sleep(30)
        now = int(time.time())
        payload = {"sent_at": now, "feeder_at": frozen if args.stalled_feeder else now}
        req = urllib.request.Request(args.url.rstrip("/") + "/v1/checkins", data=json.dumps(payload).encode(),
                                     headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
        try:
            with opener.open(req, timeout=10) as response:
                print(f"Gateway check-in {index + 1}: HTTP {response.status}", flush=True)
        except urllib.error.URLError as error:
            parser.exit(1, f"Check-in failed: {error.reason}\n")


if __name__ == "__main__":
    main()
