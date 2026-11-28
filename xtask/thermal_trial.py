# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Exercise the winter S2 controller, sensor receipt and keyboard journey."""

import json
import os
import socket
import subprocess
import sys
import tempfile
from datetime import datetime, timedelta
from pathlib import Path

from doctor import ports, running, wait_for
from playwright.sync_api import expect, sync_playwright


def main():
    command = [sys.argv[1], "thermal-trial", "--target-wh", "2300"]

    def run(*flags):
        return json.loads(subprocess.check_output([*command, *flags]))

    first = run()
    repeat = run()
    def readings(arm):
        return [{k: v for k, v in sample.items() if k != "instruction"} for sample in arm["observations"]]

    for arm in ["managed", "baseline"]:
        assert readings(first[arm]) == readings(repeat[arm])
        assert first[arm]["room"]["state"] == "met"
        assert len(first[arm]["instructions"]) == 3
    assert first["fixture_sha256"] == repeat["fixture_sha256"]
    assert first["managed"]["imported_wh"] - first["baseline"]["imported_wh"] == 125
    assert first["managed"]["exported_wh"] == first["baseline"]["exported_wh"] == 0
    assert first["managed"]["water"]["state"] == "met"
    assert first["baseline"]["water"] == {"state": "short", "minute": 15}
    assert first["equivalent_thermal_service"] is False
    boundary = json.loads(subprocess.check_output([sys.argv[1], "thermal-trial", "--target-wh", "2007"]))
    assert boundary["baseline"]["closing_shortfall_wh"] == 2.0 and not boundary["equivalent_thermal_service"]
    missing = run("--missing-close")
    assert missing["managed"]["water"] == missing["managed"]["room"] == {"state": "unknown", "minute": 15}
    morning = run("--departure-wh", "3600")
    for arm in ["managed", "baseline"]:
        departure = morning[arm]["departure"]
        assert departure["stop"] - departure["start"] == 3600
        assert departure["grid"]["import"] > 3600
    winter = json.loads(subprocess.check_output([sys.argv[1], "winter-day", "--json"]))
    assert winter["morning_fixture_sha256"] == morning["fixture_sha256"]
    for name, imported, missing_wh in [("managed", 6592, 0), ("baseline", 6467, 295)]:
        day = winter[name]
        boundaries = day["boundaries"]
        assert len(boundaries) == 97
        midnight = datetime.fromisoformat("2026-01-14T23:00:00+00:00")
        assert [datetime.fromisoformat(b["at"]) for b in boundaries] == [
            midnight + timedelta(minutes=15 * i) for i in range(97)
        ]
        for field, expected in [("import_wh", imported), ("export_wh", 3)]:
            values = [b[field] for b in boundaries]
            assert values == sorted(values) and values[-1] - values[0] == expected
        heat = day["morning"]
        ev = heat["departure"]
        assert boundaries[29]["import_wh"] - boundaries[28]["import_wh"] == heat["imported_wh"]
        assert boundaries[32]["import_wh"] - boundaries[29]["import_wh"] == ev["grid"]["import"]
        assert ev["stop"] - ev["start"] == 3600 and heat["closing_shortfall_wh"] == missing_wh
        assert (day["opening_soc"], day["closing_soc"]) == (0.30, 0.0977)
    dropped = run("--disconnect")
    assert len(dropped["managed"]["instructions"]) == 1
    assert all(s["heater_w"] == 0 for s in dropped["managed"]["observations"] if s["minute"] > 5)
    assert first["fixture_sha256"] == dropped["fixture_sha256"] == missing["fixture_sha256"]
    with tempfile.TemporaryDirectory() as directory:
        report = Path(directory) / "winter.html"
        report.write_bytes(subprocess.check_output([*command, "--html"]))
        with sync_playwright() as pw:
            chrome = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
            browser = pw.chromium.launch(executable_path=os.getenv("CHROME") or (str(chrome) if chrome.exists() else None))
            page = browser.new_page(viewport={"width": 320, "height": 740}, java_script_enabled=False)
            page.goto(report.as_uri())
            expect(page.get_by_text("Short at minute 15")).to_be_visible()
            page.keyboard.press("Tab")
            expect(page.locator("summary").first).to_be_focused()
            page.keyboard.press("Enter")
            expect(page.get_by_role("cell", name="2380.00")).to_be_visible()
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
            port, = ports(1)
            binary = str(Path(sys.argv[1]).resolve())
            with running([binary, "demo", "--listen", f"127.0.0.1:{port}"], directory, os.environ, subprocess.DEVNULL) as demo:
                def listening():
                    with socket.create_connection(("127.0.0.1", port), timeout=1):
                        return True
                wait_for(listening, demo)
                live = browser.new_page(viewport={"width": 320, "height": 740})
                live.goto(f"http://127.0.0.1:{port}/?lang=en#day")
                link = live.get_by_role("link", name="Winter trial: room comfort and hot water")
                link.focus()
                live.keyboard.press("Enter")
                expect(live.get_by_text("Short at minute 15")).to_be_visible()
                expect(live.get_by_text("6592 Wh imported", exact=True)).to_be_visible()
                expect(live.get_by_text("The baseline imported 125 Wh less.", exact=False)).to_be_visible()
                ledger = live.get_by_text("96 metered quarters", exact=True).first
                ledger.focus()
                live.keyboard.press("Enter")
                rows = ledger.locator("..").get_by_role("row")
                expect(rows).to_have_count(97)
                expect(rows.nth(1)).to_contain_text("00:00")
                expect(rows.last).to_contain_text("23:45")
                assert live.evaluate("document.documentElement.scrollWidth <= innerWidth")
                if len(sys.argv) > 2:
                    ledger.press("Enter")
                    ledger.blur()
                    live.set_viewport_size({"width": 800, "height": 900})
                    live.screenshot(path=sys.argv[2], full_page=True)
            browser.close()


if __name__ == "__main__":
    main()
