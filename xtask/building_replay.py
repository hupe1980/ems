# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Exercise the synthetic building handover through both loopback routes."""

import os
import subprocess
import sys
from contextlib import contextmanager
from pathlib import Path

from playwright.sync_api import expect, sync_playwright


@contextmanager
def demo(binary):
    with subprocess.Popen(
        [binary, "demo", "--listen", "127.0.0.1:0"], stderr=subprocess.PIPE, text=True
    ) as process:
        try:
            announcement = process.stderr.readline().strip()
            if not announcement.startswith("Open http://127.0.0.1:"):
                raise RuntimeError(f"bemsd did not announce its loopback demo: {announcement}")
            yield announcement.removeprefix("Open ").rstrip("/")
        finally:
            process.terminate()


def read(page, base, moment, case="delayed-arrival"):
    url = f"{base}/v1/demo/building?case={case}&moment={moment}"
    response = page.request.get(url)
    assert response.ok, f"{url}: expected 200, got {response.status}"
    return response.json()


def run(binary):
    with sync_playwright() as pw:
        mac_chrome = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        chrome = os.getenv("CHROME") or (str(mac_chrome) if mac_chrome.exists() else None)
        browser = pw.chromium.launch(executable_path=chrome)
        page = browser.new_page(viewport={"width": 320, "height": 700}, locale="de-DE")
        with demo(binary) as base:
            path = "/?case=delayed-arrival&moment=requested"
            page.goto(base + path)
            heading = page.locator("#zeitpunkt")
            nav = page.get_by_role("navigation", name="Zeitverlauf")
            scenarios = [
                ("requested", "Anforderung", "Prüfung ausstehend", "pending", None),
                ("observed", "Erste Messung", "Prüfung ausstehend", "pending", None),
                ("deadline", "Stichtag", "Lieferung unbestätigt", "unconfirmed", None),
                ("handover", "Übergabe", "Ladegrenze eingehalten", "within-limit", 24),
            ]
            for i, (moment, label, verdict, code, kw) in enumerate(scenarios):
                if i:
                    next_link = nav.get_by_role("link", name="Weiter: " + label)
                    page.keyboard.press("Shift+Tab")
                    expect(next_link).to_be_focused()
                    page.keyboard.press("Enter")
                    expect(heading).to_be_focused()
                expect(heading).to_contain_text(label)
                expect(page.locator("main strong")).to_have_text(verdict)
                data = read(page, base, moment)
                assert (data["moment"], data["delivery"], data["final_sample_kw"]) == (moment, code, kw), (page.url, data)
                assert (data["net_import_kw"], data["charging_kw"]) == ((52, 24) if moment == "handover" else (None, None)), page.url
                station = data["stations"][1]
                assert station["profile_accepted"] == (None if moment == "requested" else True)
                assert station["final_received_at"] == ("2026-11-22T08:13:20Z" if moment == "handover" else None)
                if moment == "observed":
                    expect(page.get_by_text("Erste Messung um 09:12:05 MEZ: 28 kW.")).to_be_visible()
                if moment == "deadline":
                    deadline_view = data
                    assert "09:13:20" not in page.content()
                if moment == "handover":
                    expect(page.get_by_text("Nachträglich bestätigt", exact=False)).to_be_visible()
                evidence = page.locator("summary", has_text="Nachweis je Ladestation")
                if i == 0:
                    page.keyboard.press("Tab")
                    expect(nav.get_by_role("link", name="Weiter: Erste Messung")).to_be_focused()
                    page.keyboard.press("Tab")
                else:
                    for _ in range(nav.get_by_role("link").count() + 1):
                        page.keyboard.press("Tab")
                expect(evidence).to_be_focused()
                page.keyboard.press("Space")
                expect(page.get_by_text("West-02", exact=True)).to_be_visible()
                page.keyboard.press("Space")
                expect(evidence).to_be_focused()
                assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"), page.url

            page.go_back()
            expect(heading).to_contain_text("Stichtag")
            page.reload()
            expect(page.locator("main strong")).to_have_text("Lieferung unbestätigt")
            page.go_forward()
            expect(heading).to_contain_text("Übergabe")
            for route in ("/", "/v1/demo/building"):
                for query in ("?moment=no-such-moment", "?case=no-such-case", "?unexpected=yes"):
                    assert page.request.get(base + route + query).status == 400, route + query
                assert page.request.post(base + route).status == 405
            for case, text in [("above-request", "26 kW gemessen"), ("unconfirmed", "Lieferung unbestätigt")]:
                page.goto(base + f"/?case={case}&moment=deadline")
                expect(page.locator("main")).to_contain_text(text)
            assert read(page, base, "deadline", "above-request")["final_sample_kw"] == 26
            for case in ("stale-charger", "unconfirmed", "idle-station"):
                data = read(page, base, "deadline", case)
                west = data["stations"][1]
                assert west["profile_accepted"] is True, case
                if case == "idle-station":
                    assert west["final_kw"] == 0 and data["delivery"] == "within-limit", data
                    assert data["final_sample_kw"] == data["stations"][0]["final_kw"], data
                else:
                    assert west["final_kw"] is None and data["delivery"] == "unconfirmed", data
                    assert data["final_sample_kw"] is None, data
            assert read(page, base, "handover", "unconfirmed")["final_sample_kw"] is None

        with demo(binary) as base:
            assert read(page, base, "deadline") == deadline_view
        browser.close()
    print("BEMS replay passed all four cursors, rewind, delayed and missing evidence, keyboard focus and cold restart.")


if __name__ == "__main__":
    run(str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/bemsd").resolve()))
