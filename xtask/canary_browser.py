# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Review signed simulator builds and their restart evidence in the operator desk."""

import json
import os
import re
import secrets
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from datetime import datetime, timedelta
from pathlib import Path
from tempfile import TemporaryDirectory
from urllib.request import Request, urlopen

from customer_portal import daemon, port, wait_ready
from playwright.sync_api import expect, sync_playwright


def main():
    with ExitStack() as stack:
        folder = Path(stack.enter_context(TemporaryDirectory()))
        password = secrets.token_hex(16)
        pg = subprocess.check_output(["docker", "run", "--rm", "-d", "-p", "127.0.0.1::5432", "-e", f"POSTGRES_PASSWORD={password}", "postgres:18-alpine"], text=True).strip()
        stack.callback(subprocess.run, ["docker", "stop", pg], check=True)
        for _ in range(150):
            if subprocess.run(["docker", "exec", pg, "pg_isready", "-h", "127.0.0.1"], check=False, stdout=subprocess.DEVNULL).returncode == 0:
                break
            time.sleep(0.1)
        db = subprocess.check_output(["docker", "port", pg, "5432/tcp"], text=True).strip()
        address = f"127.0.0.1:{port()}"
        url = "http://" + address
        config = folder / "fleet.toml"
        operator = secrets.token_hex(20)
        secret = secrets.token_hex(20)
        config.write_text(f'''[service]
listen="{address}"
[database]
url="postgres://postgres:{password}@{db}/postgres"
tls=false
[sites.haus]
enrolment_secret="{secret}"
config_version="1"
config=""
[[operators]]
token="{operator}"
tenant="*"
''')
        wait_ready(url, stack.enter_context(daemon("target/debug/fleetd", config)))
        req = Request(url + "/v1/enrol", data=json.dumps({"site": "haus", "secret": secret}).encode(), headers={"Content-Type": "application/json"})
        with urlopen(req) as response:
            token = json.load(response)["token"]
        subprocess.run(["docker", "exec", pg, "psql", "-U", "postgres", "-v", "ON_ERROR_STOP=1", "-c", "INSERT INTO enrolment SELECT 'sim-'||n,'token-'||n,now() FROM generate_series(2,100)n; INSERT INTO gateway_checkin SELECT site, t, t, t, t FROM enrolment CROSS JOIN (SELECT floor(extract(epoch FROM now()))::bigint AS t) clock"], check=True)
        pw = stack.enter_context(sync_playwright())
        chrome = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        browser = pw.chromium.launch(executable_path=os.getenv("CHROME") or (str(chrome) if chrome.exists() else None))
        stack.callback(browser.close)
        page = browser.new_page(viewport={"width": 320, "height": 740}, has_touch=True)
        page.set_extra_http_headers({"Authorization": "Bearer " + operator})
        page.goto(url + "/desk")
        # One feeder stopped advancing eight minutes ago. A receipt without
        # a feeder reading and an irregular gap exercise the plotted time axis.
        subprocess.run(["docker", "exec", pg, "psql", "-U", "postgres", "-v", "ON_ERROR_STOP=1", "-c", """
            WITH origin AS (SELECT sent_at AS t FROM gateway_checkin WHERE site='sim-2')
            INSERT INTO gateway_checkin
            SELECT 'sim-2', t-n*30, t-n*30+2, CASE WHEN n<>10 THEN least(t-n*30-4,t-480) END, NULL
            FROM origin CROSS JOIN generate_series(1,31)n WHERE n NOT BETWEEN 19 AND 22;
            UPDATE gateway_checkin SET feeder_at=sent_at-480, report_at=NULL
            WHERE site='sim-2' AND sent_at=(SELECT max(sent_at) FROM gateway_checkin WHERE site='sim-2')
        """], check=True)
        expect(page.locator("#position")).to_have_text("1–15 of 100")
        page.locator("#next").press("Enter")
        expect(page.locator("#position")).to_have_text("16–30 of 100")
        page.reload()
        expect(page.locator("#position")).to_have_text("16–30 of 100")
        page.go_back()
        expect(page.locator("#position")).to_have_text("1–15 of 100")
        page.get_by_role("button", name="Refresh desk").click()
        page.locator("#search").fill("sim-2")
        page.get_by_text("Needs attention", exact=True).click()
        card = page.locator("article").filter(has_text="Inspect feeder").first
        card.locator("summary").first.press("Enter")
        trend = card.locator(".trend")
        expect(trend.locator(".dot")).to_have_count(27)
        expect(trend.locator(".scale")).to_contain_text("27 of 28")
        expect(trend.locator(".limit")).to_be_visible()
        cursor = trend.get_by_role("slider")
        cursor.press("Home")
        expect(trend.locator("output")).to_contain_text("4 s")
        cursor.press("End")
        expect(trend.locator("output")).to_contain_text("480 s")
        metric = trend.get_by_role("combobox")
        metric.select_option("report_at")
        expect(trend.locator(".dot")).to_have_count(0)
        expect(trend.locator("output")).to_contain_text("Unknown")
        metric.select_option("sent_at")
        expect(trend.locator(".dot")).to_have_count(28)
        metric.select_option("feeder_at")
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        if len(sys.argv) > 2:
            page.set_viewport_size({"width": 1120, "height": 880})
            page.evaluate("window.scrollTo(0, 0)")
            page.screenshot(path=sys.argv[2], full_page=True)
            page.set_viewport_size({"width": 320, "height": 740})
        subprocess.run(["docker", "exec", pg, "createdb", "-U", "postgres", "history"], check=True)
        hist = f"127.0.0.1:{port()}"
        meter_key = secrets.token_hex(20)
        hist_config = folder / "hist.toml"
        hist_config.write_text(f'''[service]
listen="{hist}"
[database]
url="postgres://postgres:{password}@{db}/history"
tls=false
[site_tokens]
sim-2="{meter_key}"
[tenants]
desk=["sim-2"]
[[operators]]
token="{operator}"
tenant="desk"
''')
        wait_ready("http://" + hist, stack.enter_context(daemon("target/debug/histd", hist_config)))
        # An overnight import peak, afternoon export, one corrected quarter and a gap.
        start = datetime.fromisoformat("2025-10-25T22:00:00+00:00")
        assertions = []
        for i in range(100):
            if 32 <= i < 38:
                continue
            slot = start + timedelta(minutes=i * 15)
            reading = {"slot": slot.isoformat(), "grid_draw": str(round(max(0, 1.4 - abs(i-22)*0.06), 3)),
                       "grid_feed_in": str(round(max(0, 1.6 - abs(i-56)*0.11), 3)),
                       "device_consumption": "0", "device_generation": "0"}
            if i in (23, 24):
                reading["grid_draw"] = "1.4000001"
            assertions.append({"recorded_at": (slot + timedelta(minutes=15)).isoformat(), "registers": reading})
        assertions.append({"recorded_at": (start + timedelta(days=2)).isoformat(),
                           "registers": {**assertions[0]["registers"], "grid_draw": "0.1234567"}})
        assertions.append({"recorded_at": start.isoformat(),
                           "registers": {**assertions[0]["registers"],
                                         "slot": (start - timedelta(minutes=15)).isoformat(),
                                         "grid_draw": "-0.1", "grid_feed_in": "77"}})
        zero_slot = start - timedelta(days=2)
        assertions.append({"recorded_at": (zero_slot + timedelta(minutes=15)).isoformat(),
                           "registers": {**assertions[0]["registers"], "slot": zero_slot.isoformat(),
                                         "grid_draw": "0", "grid_feed_in": "0"}})
        # Six weeks of quarters before the separate one-day fault fixtures.
        for offset in range(3, 45):
            for quarter in range(96):
                slot = start - timedelta(days=offset) + timedelta(minutes=quarter*15)
                solar = max(0, 1 - abs(quarter-50)/20)
                load = max(0, 1 - abs(quarter-74)/12)
                assertions.append({"recorded_at": (slot + timedelta(minutes=15)).isoformat(),
                                   "registers": {**assertions[0]["registers"], "slot": slot.isoformat(),
                                                 "grid_draw": str(round(.2 + load, 3)), "grid_feed_in": str(round(solar, 3))}})
        for offset in range(0, len(assertions), 128):
            req = Request("http://" + hist + "/v1/sites/sim-2/quarter-hours",
                          data=json.dumps(assertions[offset:offset+128]).encode(),
                          headers={"Content-Type": "application/json", "Authorization": "Bearer " + meter_key})
            with urlopen(req) as response:
                assert response.status == 204
        # Dispatch this same-origin URL to the real historian as the deployment ingress does.
        page.route("**/v1/desk/energy?*", lambda route: route.fulfill(
            response=route.fetch(url="http://" + hist + "/v1/desk/energy?" + route.request.url.split("?", 1)[1])))
        energy = card.locator(".energy")
        energy.locator("summary").first.press("Enter")
        energy.get_by_label("History range").select_option("1")
        energy.get_by_label("Period ending").fill("2025-10-26")
        expect(energy).to_contain_text("94 of 100 closed quarters measured")
        chart = energy.get_by_role("slider")
        chart.press("Home")
        expect(chart).to_have_attribute("aria-valuetext", re.compile("Import 0.1234567 kWh"))
        for label, slot, text in [
            ("Highest measured import", 24, "04:45 GMT+1 · Import 1.4000001 kWh"),
            ("Highest measured export", 57, "Export 1.6 kWh"),
            ("First missing reading", 33, "07:00 GMT+1 · Missing"),
        ]:
            energy.get_by_role("button", name=re.compile(label)).press("Enter")
            expect(chart).to_be_focused()
            expect(chart).to_have_attribute("aria-valuenow", str(slot))
            expect(chart).to_have_attribute("aria-valuetext", re.compile(re.escape(text)))
        expect(energy.get_by_role("button", name=re.compile("First missing reading"))).to_contain_text("1 h 30 min gap")
        expect(energy.locator(".missing")).to_have_count(6)
        expect(energy.locator("table")).to_contain_text("GMT+2")
        expect(energy.locator("table")).to_contain_text("GMT+1")
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        if len(sys.argv) > 3:
            page.set_viewport_size({"width": 1000, "height": 800})
            energy.screenshot(path=sys.argv[3])
            page.set_viewport_size({"width": 320, "height": 740})
        period = energy.get_by_label("History range")
        period.select_option("90")
        long = energy.locator("meter-history")
        expect(long.locator("canvas")).to_be_visible()
        expect(long.locator(".coverage")).to_contain_text("4,127 of 8,644 closed quarters measured")
        cursor = long.get_by_role("slider")
        overlay = long.locator(".u-over")
        overlay.tap(position={"x": 12, "y": 32})
        assert 1 < int(cursor.get_attribute("aria-valuenow")) < 4322
        long.get_by_role("button", name="Peak import").press("Enter")
        expect(cursor).to_have_attribute("aria-valuetext", re.compile(r"04:45 GMT\+1 · Import 1.4000001 kWh"))
        long.get_by_role("button", name="Zoom to day").press("Enter")
        expect(long.get_by_role("button", name="Reset zoom")).to_be_enabled()
        cursor.press("Home")
        expect(cursor).to_have_attribute("aria-valuetext", re.compile(r"26 Oct 2025.*00:00 GMT\+2.*0.1234567 kWh"))
        long.get_by_role("button", name="Open meter day").press("Enter")
        expect(period).to_have_value("1")
        expect(energy.get_by_role("slider")).to_be_focused()
        expect(energy.get_by_role("slider")).to_have_attribute("aria-valuetext", re.compile("0.1234567 kWh"))
        energy.get_by_role("button", name="Return to range").press("Enter")
        expect(period).to_have_value("90")
        expect(cursor).to_be_focused()
        expect(cursor).to_have_attribute("aria-valuetext", re.compile(r"26 Oct 2025.*00:00 GMT\+2"))
        reset = long.get_by_role("button", name="Reset zoom")
        expect(reset).to_be_enabled()
        reset.press("Enter")
        expect(reset).to_be_disabled()
        cursor.press("Home")
        cursor.press("ArrowRight")
        expect(cursor).to_have_attribute("aria-valuenow", "2")
        long.get_by_role("button", name="First gap").press("Enter")
        expect(cursor).to_have_attribute("aria-valuetext", re.compile("Missing reading"))
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        period.select_option("30")
        expect(cursor).to_have_attribute("aria-valuemax", "2884")
        page.set_viewport_size({"width": 1100, "height": 860})
        overlay = long.locator(".u-over")
        overlay.scroll_into_view_if_needed()
        box = overlay.bounding_box()
        y = box["y"] + box["height"] / 2
        page.mouse.move(box["x"] + box["width"] / 4, y)
        page.mouse.down()
        page.mouse.move(box["x"] + box["width"] / 2, y, steps=8)
        page.mouse.up()
        expect(reset).to_be_enabled()
        cursor.press("Home")
        assert int(cursor.get_attribute("aria-valuenow")) > 1
        reset.press("Enter")
        expect(reset).to_be_disabled()
        long.get_by_role("button", name="Peak import").click()
        if len(sys.argv) > 4:
            energy.screenshot(path=sys.argv[4])
        page.set_viewport_size({"width": 320, "height": 740})
        period.select_option("1")
        energy.get_by_label("Period ending").fill("2025-03-30")
        expect(energy).to_contain_text("0 of 92 closed quarters measured")
        expect(energy.get_by_role("button", name=re.compile("Highest measured"))).to_have_count(0)
        expect(energy.get_by_role("button", name=re.compile("First missing reading"))).to_contain_text("23 h gap")
        energy.get_by_label("Period ending").fill("2025-10-25")
        expect(energy.locator(".invalid")).to_have_count(1)
        expect(energy).to_contain_text("Invalid · Import -0.1 kWh")
        expect(energy.get_by_role("button", name=re.compile("Highest measured"))).to_have_count(0)
        energy.get_by_label("Period ending").fill("2025-10-24")
        expect(energy).to_contain_text("1 of 96 closed quarters measured")
        expect(energy.get_by_role("button", name=re.compile("Highest measured"))).to_have_count(0)
        chart.press("Home")
        expect(chart).to_have_attribute("aria-valuetext", re.compile("Import 0 kWh · Export 0 kWh"))
        page.unroute("**/v1/desk/energy?*")
        energy.get_by_label("Period ending").fill("2025-10-23")
        expect(energy.get_by_role("button", name="Retry meter history")).to_be_visible()
        page.locator("#search").fill("")
        page.get_by_text("All sites", exact=True).click()
        for case, decision in [("wrong-target", "wrong target"), ("rejected", "verification rejected"), ("rollback", "rolled back"), ("wrong-build", "different build"), ("unready", "gateway unready"), ("ready", "Qualified")]:
            subprocess.run([sys.executable, "xtask/canary_trial.py", "--config", config, "--url", url, "--site", "haus", "--case", case], env={**os.environ, "FLEET_SITE_TOKEN": token}, check=True)
            button = page.get_by_role("button", name="Refresh desk")
            button.focus()
            button.press("Enter")
            expect(page.locator("#canary-decision")).to_contain_text(decision)
            expect(button).to_be_focused()
            expect(page.locator("#summary")).to_contain_text("100 enrolled")
        subprocess.run(["docker", "exec", pg, "psql", "-U", "postgres", "-v", "ON_ERROR_STOP=1", "-c", "INSERT INTO enrolment VALUES ('later', 'new-token', now())"], check=True)
        page.get_by_role("button", name="Refresh desk").click()
        expect(page.locator("#summary")).to_contain_text("101 enrolled")
        page.get_by_text("Rehearsal cohort", exact=True).click()
        expect(page.locator("#position")).to_contain_text("of 100")
        page.locator("#search").fill("later")
        expect(page.locator("#position")).to_have_text("No matching sites")
        page.locator("#search").fill("")
        page.locator("#canary summary").focus()
        page.keyboard.press("Enter")
        expect(page.locator("#canary-evidence")).to_contain_text("100 visible sites")
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        if len(sys.argv) > 1:
            page.set_viewport_size({"width": 1120, "height": 800})
            page.screenshot(path=sys.argv[1])
        # Measure a populated roster through the real API before checking the browser page.
        roster_size = int(os.environ.get("EMS_DESK_SCALE_SITES", "10000"))
        assert 102 <= roster_size <= 1_000_000
        added = roster_size - 101
        subprocess.run(["docker", "exec", pg, "psql", "-U", "postgres", "-v", "ON_ERROR_STOP=1", "-c", f"""
            INSERT INTO enrolment SELECT 'scale-'||n,'scale-token-'||n,now() FROM generate_series(1,{added})n;
            INSERT INTO gateway_checkin
                SELECT site,t-i*30,t-i*30,t-i*30,t-i*30 FROM enrolment
                CROSS JOIN (SELECT floor(extract(epoch FROM now()))::bigint AS t) clock
                CROSS JOIN generate_series(0,7)i WHERE site LIKE 'scale-%';
            ANALYZE enrolment; ANALYZE gateway_checkin;
        """], check=True)
        def browse(index, exact=False):
            started = time.monotonic()
            query = f"scale-{added}" if exact else "scale-"
            req = Request(url + "/v1/desk?q=" + query + "&page=" + str(index), headers={"Authorization": "Bearer " + operator})
            with urlopen(req, timeout=30) as response:
                body = response.read()
            data = json.loads(body)
            assert data["total"] == data["matched"] == (1 if exact else added)
            assert 0 < len(data["rows"]) <= 15
            return (time.monotonic() - started) * 1000, len(body)
        with ThreadPoolExecutor(max_workers=8) as workers:
            samples = sorted(workers.map(browse, range(24)))
            exact = sorted(workers.map(lambda i: browse(i, True), range(24)))
        print(f"{roster_size} sites, 8 concurrent readers, 24 HTTP requests: median {samples[12][0]:.0f} ms, max {samples[-1][0]:.0f} ms, max {max(s[1] for s in samples)} bytes.")
        print(f"Exact site search: median {exact[12][0]:.0f} ms, max {exact[-1][0]:.0f} ms, max {max(s[1] for s in exact)} bytes.")
        page.get_by_text("All sites", exact=True).click()
        page.locator("#search").fill(f"scale-{added}")
        expect(page.locator("article summary").first).to_contain_text(f"scale-{added}")
        expect(page.locator("article")).to_have_count(1)
        page.set_extra_http_headers({})
        page.get_by_role("button", name="Refresh desk").click()
        expect(page.locator("#canary")).to_be_hidden()


if __name__ == "__main__":
    main()
