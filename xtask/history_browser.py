# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Exercise the shipped household page against deterministic history responses.

Run with `uv run xtask/history_browser.py`. Set CHROME
to an installed Chromium executable, or install Playwright's Chromium.
"""

import json
import os
from datetime import datetime, timedelta
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread
from urllib.parse import parse_qs, urlsplit

from playwright.sync_api import expect, sync_playwright


class Assets(SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


def exercise(page, base):
    outcomes = {"grid": 200, "netzwirksam": 200, "wallbox": 503}
    queries = []

    def respond(route):
        url = urlsplit(route.request.url)
        if url.path == "/v1/home":
            return route.fulfill(json={"site": "Garden house", "devices": [{"id": "wallbox"}]})
        if url.path == "/v1/days":
            bill = {"energy_ct": 99, "fixed_ct": 1, "expected_intervals": 96, "priced_intervals": 96}
            observed = {"metered_bill": bill, "imported_wh": 1000, "produced_wh": 0, "reductions": 0}
            equal = {"with_plan_ct": 100, "without_plan_ct": 100, "error_ct": 1}
            return route.fulfill(json={"days": [
                {**observed, "day": "2026-11-21", "comparison": equal},
                {**observed, "day": "2026-11-22", "comparison": None,
                 "comparison_gap": {"at_unix_s": 1795343400, "reason": "unverified_service"}},
            ]})
        point = url.path.rsplit("/", 1)[-1]
        assert url.path.startswith("/v1/series/") and point in outcomes, url.path
        query = parse_qs(url.query)
        assert query["resolution"] == ["quarter_hours"]
        start, end = [datetime.fromisoformat(query[k][0].replace("Z", "+00:00")) for k in ("from", "to")]
        assert start.timestamp() % 900 == end.timestamp() % 900 == 0 and start < end
        queries.append((point, start, end))
        status = outcomes[point]
        if status != 200:
            return route.fulfill(status=status, json={"status": status})
        rows = [
            {"at": start.isoformat(), "watts": 0, "min_watts": 0, "max_watts": 0, "samples": 1},
            {"at": (start + timedelta(minutes=15)).isoformat(), "watts": 300, "min_watts": -200, "max_watts": 7000, "samples": 18},
            {"at": (start + timedelta(minutes=45)).isoformat(), "watts": 0.25},
        ]
        return route.fulfill(json={"resolution": "quarter_hours", "readings": [] if point == "netzwirksam" else rows})

    page.route("**/v1/**", respond)
    page.clock.set_fixed_time(datetime.fromisoformat("2026-11-23T12:00:00+01:00"))
    page.goto(base + "/?lang=en#day")
    expect(page.locator(".month")).to_contain_text("Equivalent service verified on 1 of 2 recorded days")
    page.get_by_role("button", name="2026-11-21").press("Enter")
    receipt = page.locator(".receipt")
    expect(receipt).to_contain_text("€0.00 · Within model error")
    page.get_by_role("button", name="2026-11-22").press("Enter")
    expect(receipt).to_contain_text("Equivalent service unverified")
    expect(receipt).to_contain_text("GMT+1")
    inspector = page.locator("#gridHistory")
    summary = inspector.locator("summary")
    summary.focus()
    summary.press("Enter")
    result = inspector.get_by_role("status")
    expect(result).to_contain_text("No reading was recorded")
    day, quarter = inspector.get_by_label("Date in Berlin"), inspector.get_by_label("Quarter hour starting")
    for date, count in [("2026-03-29", 92), ("2026-10-25", 100)]:
        day.fill(date)
        expect(quarter).to_be_enabled()
        expect(quarter.locator("option")).to_have_count(count)
        assert queries[-1][2] - queries[-1][1] == timedelta(minutes=count * 15)
    repeated = quarter.locator("option").all_text_contents()
    assert len({label for label in repeated if label.startswith("02:00")}) == 2
    quarter.focus()
    quarter.select_option(index=0)
    expect(result.locator("dd")).to_have_text(["0 W", "0 W", "0 W", "1"])
    quarter.select_option(index=1)
    expect(result.locator("dd")).to_have_text(["300 W", "-200 W", "7,000 W", "18"])
    page.evaluate("load()")
    expect(quarter).to_be_focused()
    quarter.select_option(index=2)
    expect(result).to_contain_text("No reading was recorded")
    quarter.select_option(index=3)
    expect(result.locator("dd")).to_have_text(["0.25 W", "Unavailable", "Unavailable", "Unavailable"])
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")

    outcomes["grid"] = 503
    day.fill("2026-10-24")
    expect(result).to_contain_text("another history query")
    outcomes["grid"] = 500
    result.get_by_role("button").click()
    expect(result).to_contain_text("could not be loaded")
    outcomes["grid"] = 404
    result.get_by_role("button").click()
    expect(result).to_have_text("Local measurement history is not configured.")
    outcomes["grid"] = 200
    button = page.get_by_role("button", name="Download local power history")
    with page.expect_download() as downloaded:
        button.click()
    report = json.loads(downloaded.value.path().read_text())
    assert downloaded.value.suggested_filename.endswith("-partial.json")
    assert report["complete"] is False and report["failures"] == [{"point": "wallbox", "status": 503}]
    assert set(report["series"]) == {"grid", "netzwirksam"}
    assert report["series"]["netzwirksam"]["readings"] == []
    expect(page.locator("#exportStatus")).to_contain_text("Missing points: wallbox")
    outcomes["wallbox"] = 200
    with page.expect_download() as downloaded:
        button.click()
    report = json.loads(downloaded.value.path().read_text())
    assert report["complete"] and report["failures"] == [] and "wallbox" in report["series"]
    outcomes.update(dict.fromkeys(outcomes, 500))
    downloads = []
    page.on("download", lambda download: downloads.append(download))
    button.click()
    expect(page.locator("#exportStatus")).to_contain_text("No file created")
    assert not downloads
    outcomes.update(dict.fromkeys(outcomes, 404))
    button.click()
    expect(page.locator("#exportStatus")).to_have_text("Local measurement history is not configured.")
    assert not downloads
    outcomes["grid"] = 500
    page.goto(base + "/?lang=de#day")
    page.get_by_text("Netzanschluss · Viertelstunde prüfen").click()
    expect(page.locator(".history-result")).to_contain_text("konnten nicht geladen werden")
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")


def main():
    ui = Path(__file__).resolve().parents[1] / "services/hemsd/ui"
    default = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    with ThreadingHTTPServer(("127.0.0.1", 0), partial(Assets, directory=str(ui))) as server:
        Thread(target=server.serve_forever, daemon=True).start()
        try:
            with sync_playwright() as pw:
                executable = os.getenv("CHROME") or (str(default) if default.exists() else None)
                browser = pw.chromium.launch(executable_path=executable)
                page = browser.new_page(viewport={"width": 320, "height": 720})
                exercise(page, f"http://127.0.0.1:{server.server_port}")
                browser.close()
        finally:
            server.shutdown()
    print("Passed quarter inspection, Berlin clock changes, keyboard input and JSON downloads.")


if __name__ == "__main__":
    main()
