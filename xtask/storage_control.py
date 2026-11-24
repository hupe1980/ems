# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Check confirmed, lost and recovered battery control in the household view."""
import os
import re
import secrets
import sys
import tempfile
from pathlib import Path

from playwright.sync_api import Error, expect, sync_playwright

from doctor import ports, running, wait_for


def run(binary, directory):
    api, modbus, ship = ports(3)
    config = directory / "house.toml"
    token = secrets.token_hex(24)
    env = dict(os.environ, EMS_STORAGE_TRIAL=token)
    peer = [binary, "house", "--manage-storage", "--at", "12:00",
            "--listen", f"127.0.0.1:{modbus}", "--api", f"127.0.0.1:{api}",
            "--ship", f"127.0.0.1:{ship}", "--out", str(config)]
    with (directory / "trial.log").open("w") as log, running(peer, directory, env, log) as house:
        generated = wait_for(config.read_text, house)
        gateway_file = directory / "gateway.toml"
        gateway_file.write_text('[api]\ntoken = "env:EMS_STORAGE_TRIAL"\n' + generated)
        with running([binary, "run", "--config", str(gateway_file)], directory, env, log) as gateway, sync_playwright() as pw:
            mac = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
            browser = pw.chromium.launch(executable_path=os.getenv("CHROME") or (str(mac) if mac.exists() else None))
            page = browser.new_page(viewport={"width": 320, "height": 740}, locale="en-GB")
            base = f"http://127.0.0.1:{api}"

            def battery():
                try:
                    response = page.request.get(base + "/v1/home", headers={"Authorization": "Bearer " + token})
                except Error:
                    if gateway.poll() is not None:
                        raise
                    return None
                if not response.ok:
                    return None
                return next((d for d in response.json()["devices"] if d.get("battery_control")), None)

            def confirmed():
                device = battery()
                return device and device["measured_w"] is not None and device["battery_control"]["confirmed_w"] is not None

            wait_for(confirmed, gateway)
            assert page.request.post(base + "/v1/session", data={"token": token}).ok
            page.goto(base)
            summary = page.locator("#packControl summary")
            summary.focus()
            page.keyboard.press("Enter")
            panel = page.locator("#packControl")
            expect(panel).to_have_attribute("open", "")
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
            announcement = page.locator("#packAnnouncement")
            expect(announcement).to_contain_text(re.compile("Confirmed|bestätigt"))
            house.terminate()
            house.wait(timeout=15)

            def lost():
                d = battery()
                return d and d["measured_w"] is None and d["battery_control"]["confirmed_w"] is None and d["battery_control"]["last_confirmed"]

            wait_for(lost, gateway)
            expect(panel).to_contain_text(re.compile("Unknown|Unbekannt"), timeout=15000)
            expect(announcement).to_contain_text(re.compile("Unknown|Unbekannt"))
            expect(summary).to_be_focused()
            with running(peer, directory, env, log):
                wait_for(confirmed, gateway)
                expect(announcement).to_contain_text(re.compile("Confirmed|bestätigt"), timeout=15000)
            browser.close()
    print("Managed battery confirmed, then lost and recovered; keyboard detail stayed open at 320 px.")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as directory:
        run(str(Path(sys.argv[1]).resolve()), Path(directory))
