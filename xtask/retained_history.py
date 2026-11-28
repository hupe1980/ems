# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Export a populated Chronix snapshot from Day while hemsd controls a virtual house."""

import json
import os
import platform
import secrets
import shutil
import ssl
import subprocess
import sys
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, urlencode, urlsplit
from urllib.request import Request, urlopen

from playwright.sync_api import sync_playwright

from doctor import ports, running, wait_for

QUARTERS = 1_095 * 96
POINTS = ("grid", "netzwirksam", "battery", "pv")


def iso(seconds):
    return datetime.fromtimestamp(seconds, UTC).isoformat()


def logical_bytes(path):
    return sum(item.stat().st_size for item in path.rglob("*") if item.is_file())


def check_download(report, start, end, expected_points):
    assert report["complete"] and report["failures"] == [], report["failures"]
    assert datetime.fromisoformat(report["from"]).timestamp() == start
    assert datetime.fromisoformat(report["to"]).timestamp() == end
    assert set(report["series"]) == expected_points, expected_points
    for channel, point in enumerate(POINTS):
        series = report["series"][point]
        assert series["resolution"] == "quarter_hours"
        rows = series["readings"]
        assert len(rows) == QUARTERS - (point == "grid"), (point, len(rows))
        # Validate the entire ordered time axis and the data, including the absent
        # third grid quarter. This catches duplicates, a wrong tag and partial windows.
        for i, row in enumerate(rows):
            quarter = i + int(point == "grid" and i >= 2)
            assert datetime.fromisoformat(row["at"]).timestamp() == start + 900 * quarter
            value = 100 + (quarter + channel) % 97
            if point == "pv":
                value = -value
            expected = (value, value, value, 1)
            if point == "grid" and quarter == 0:
                expected = (1_000, -500, 2_500, 2)
            if point == "grid" and quarter == 1:
                expected = (0, 0, 0, 1)
            actual = tuple(row[k] for k in ("watts", "min_watts", "max_watts", "samples"))
            assert actual == expected, (point, quarter, actual, expected)
    for point in expected_points.difference(POINTS):
        assert report["series"][point]["readings"] == [], point


def inspect(base, ctx, token, start, end, database_bytes):
    def get(path):
        req = Request(base + path, headers={"Authorization": f"Bearer {token}"})
        with urlopen(req, context=ctx, timeout=180) as response:
            return json.load(response)

    def history_url(first, last):
        return "/v1/series/grid?" + urlencode(
            {"from": iso(first), "to": iso(last), "resolution": "quarter_hours"}
        )

    home = get("/v1/home")
    expected_points = {"grid", "netzwirksam", *(d["id"] for d in home["devices"])}
    readings = get(history_url(start, start + 2700))["readings"]
    assert len(readings) == 2
    assert [r["samples"] for r in readings] == [2, 1]
    assert readings[0]["max_watts"] == 2500 and readings[1]["watts"] == 0
    assert get(history_url(start + 1800, start + 2700))["readings"] == []

    initial = get("/v1/status")
    barrier = threading.Barrier(2, timeout=10)

    def concurrent_read():
        barrier.wait()
        try:
            return 200, len(get(history_url(start, end))["readings"])
        except HTTPError as exc:
            return exc.code, 0

    with ThreadPoolExecutor(max_workers=2) as workers:
        futures = [workers.submit(concurrent_read) for _ in range(2)]
        assert sorted(f.result() for f in futures) == [(200, QUARTERS - 1), (503, 0)]

    with sync_playwright() as pw:
        default = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        executable = os.getenv("CHROME") or (str(default) if default.exists() else None)
        browser = pw.chromium.launch(executable_path=executable)
        page = browser.new_page(ignore_https_errors=True, viewport={"width": 320, "height": 720})
        assert page.request.post(base + "/v1/session", data={"token": token}).ok
        # Freeze the export's interval at the snapshot boundary. The separate
        # gateway and its control clock keep operating in real time.
        page.clock.set_fixed_time(datetime.fromtimestamp(end, UTC))
        page.goto(base + "/?lang=en#day")
        button = page.get_by_role("button", name="Download local power history")
        button.wait_for()
        latencies = []

        def completed(request):
            url = urlsplit(request.url)
            starts = parse_qs(url.query).get("from", [])
            if url.path.startswith("/v1/series/") and starts:
                if datetime.fromisoformat(starts[0]).timestamp() == start:
                    latencies.append(request.timing["responseEnd"])

        page.on("requestfinished", completed)
        before = get("/v1/status")
        tick_stamps = {before["at"]}
        stop = threading.Event()

        def observe():
            while not stop.wait(0.2):
                tick_stamps.add(get("/v1/status")["at"])

        with ThreadPoolExecutor(max_workers=1) as watcher:
            sampling = watcher.submit(observe)
            try:
                began = time.monotonic()
                with page.expect_download(timeout=300_000) as download:
                    button.click()
                path = download.value.path()
                duration = time.monotonic() - began
            finally:
                stop.set()
            sampling.result()
        final = get("/v1/status")
        tick_stamps.add(final["at"])
        assert len(tick_stamps) >= 2, "No control tick observed during the browser download"
        assert final["overruns"] == initial["overruns"], (initial["overruns"], final["overruns"])
        assert len(latencies) == len(expected_points) and min(latencies) >= 0, latencies
        check_download(json.loads(path.read_bytes()), start, end, expected_points)
        print(json.dumps({
            "runner": platform.platform(), "cpu_count": os.cpu_count(), "days": 1095,
            "quarters_per_point": QUARTERS, "seeded_points": len(POINTS),
            "exported_points": len(expected_points), "exported_rows": 4 * QUARTERS - 1,
            "database_bytes": database_bytes, "download_bytes": path.stat().st_size,
            "download_s": round(duration, 3),
            "slowest_series_request_ms": round(max(latencies), 1),
            "distinct_control_ticks_during_download": len(tick_stamps),
            "new_control_overruns": final["overruns"] - initial["overruns"],
            "concurrent_read_status": [200, 503], "snapshot_end": iso(end),
        }, indent=2), flush=True)
        browser.close()


def export_snapshot(binary, directory, end):
    start = end - 900 * QUARTERS
    store = directory / "series"
    database_bytes = logical_bytes(store)
    assert database_bytes > 0, "The imported Chronix snapshot must contain files"
    print(f"Closed snapshot: {database_bytes:,} logical bytes", flush=True)
    api, modbus, ship = ports(3)
    token = secrets.token_hex(24)
    env = dict(os.environ, EMS_HISTORY_TRIAL=token)
    config = directory / "house.toml"
    base = f"https://localhost:{api}"
    with (directory / "trial.log").open("w") as log, running([
        binary, "house", "--listen", f"127.0.0.1:{modbus}", "--ship", f"127.0.0.1:{ship}",
        "--api", f"127.0.0.1:{api}", "--out", str(config)
    ], directory, env, log) as house:
        written = wait_for(config.read_text, house)
        config.write_text('[api]\ntls = true\ntoken = "env:EMS_HISTORY_TRIAL"\n[series]\npath = "series"\n' + written)
        with running([binary, "run", "--config", str(config)], directory, env, log) as gateway:
            pem = wait_for(lambda: ssl.get_server_certificate(("127.0.0.1", api), timeout=5), gateway)
            ctx = ssl.create_default_context(cadata=pem)

            def ready():
                req = Request(base + "/v1/status", headers={"Authorization": f"Bearer {token}"})
                try:
                    with urlopen(req, context=ctx, timeout=5) as response:
                        return json.load(response)["at"]
                except (ConnectionError, URLError):
                    return None

            wait_for(ready, gateway)
            inspect(base, ctx, token, start, end, database_bytes)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: uv run xtask/retained_history.py HEMSD SEED_HISTORY")
    with tempfile.TemporaryDirectory() as workspace:
        directory = Path(workspace)
        # Cargo can replace target/debug/hemsd during the long fixture import.
        binary = shutil.copy(sys.argv[1], directory / Path(sys.argv[1]).name)
        subprocess.run([binary, "house", "--help"], check=True, stdout=subprocess.DEVNULL)
        seeder = Path(sys.argv[2]).resolve()
        end = int(time.time()) // 3600 * 3600
        subprocess.run([seeder, directory / "series", str(end)], check=True)
        export_snapshot(binary, directory, end)
