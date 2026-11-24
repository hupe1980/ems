# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Rehearse a household receipt through both daemons and the OCPP simulator."""
import json
import os
import secrets
import socket
import subprocess
import sys
import time
from contextlib import ExitStack, closing, contextmanager
from pathlib import Path
from tempfile import TemporaryDirectory
from urllib.request import urlopen

from playwright.sync_api import expect, sync_playwright


def port():
    with closing(socket.socket()) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_ready(url, process):
    for _ in range(150):
        if process.poll() is not None:
            raise RuntimeError(f"Service exited before serving {url}")
        try:
            with urlopen(url + "/readyz", timeout=1) as response:
                if response.status == 200:
                    return
        except OSError:
            time.sleep(0.1)
    raise TimeoutError(url)


@contextmanager
def daemon(binary, config):
    process = subprocess.Popen([binary, "--config", config])
    try:
        yield process
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def run(hemsd, obsd, portald):
    with ExitStack() as stack:
        directory = Path(stack.enter_context(TemporaryDirectory()))
        password = secrets.token_hex(16)
        pg = subprocess.check_output([
            "docker", "run", "--rm", "-d", "-p", "127.0.0.1::5432",
            "-e", f"POSTGRES_PASSWORD={password}", "postgres:18-alpine"
        ], text=True).strip()
        stack.callback(subprocess.run, ["docker", "stop", pg], check=True)
        for _ in range(150):
            if subprocess.run(["docker", "exec", pg, "pg_isready", "-h", "127.0.0.1"],
                              stdout=subprocess.DEVNULL).returncode == 0:
                break
            time.sleep(0.1)
        else:
            raise TimeoutError("PostgreSQL did not start")
        pg_addr = subprocess.check_output(["docker", "port", pg, "5432/tcp"], text=True).strip()
        collect_addr = f"127.0.0.1:{port()}"
        collect = "http://" + collect_addr
        keys = {site: secrets.token_hex(20) for site in ["home", "second"]}
        tokens = {site: secrets.token_hex(20) for site in keys}
        config = directory / "collector.toml"
        config.write_text(f'''[service]
listen = "{collect_addr}"
[database]
url = "postgres://postgres:{password}@{pg_addr}/postgres"
tls = false
[webhook_secrets]
home = ["{keys['home']}"]
second = ["{keys['second']}"]
[customer_tokens]
home = "{tokens['home']}"
second = "{tokens['second']}"
''')
        collector = stack.enter_context(daemon(obsd, config))
        wait_ready(collect, collector)
        address = f"127.0.0.1:{port()}"
        base = "http://" + address
        portal_config = directory / "portal.toml"
        portal_config.write_text(f'collector_origin="{collect}"\n[service]\nlisten="{address}"\n')
        wait_ready(base, stack.enter_context(daemon(portald, portal_config)))

        def send(site, goal, *flags):
            subprocess.run([hemsd, "ev-departure", "--site", site, "--goal-wh", str(goal),
                            "--report-to", collect, *flags], check=True,
                           env={**os.environ, "EMS_OBSD_SECRET": keys[site]})

        pw = stack.enter_context(sync_playwright())
        local = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        chrome = os.getenv("CHROME") or (str(local) if local.exists() else None)
        browser = pw.chromium.launch(executable_path=chrome)
        stack.callback(browser.close)
        page = browser.new_page(viewport={"width": 320, "height": 740})
        subprocess.run(["docker", "exec", pg, "psql", "-U", "postgres", "-v", "ON_ERROR_STOP=1", "-c",
            """INSERT INTO site_day VALUES
               ('home','2026-01-14','{"imported_kwh":8,"exported_kwh":1,"produced_kwh":4}',now()),
               ('home','2026-01-12','{"imported_kwh":0,"exported_kwh":0,"produced_kwh":0}',now()),
               ('home','2026-01-01','{"imported_kwh":3,"exported_kwh":0,"produced_kwh":1}',now()),
               ('second','2026-01-13','{"imported_kwh":42,"exported_kwh":2,"produced_kwh":5}',now())"""], check=True)
        page.set_extra_http_headers({"Authorization": "Bearer " + tokens["home"]})
        page.goto(base)
        expect(page).to_have_url(base + "/portal")
        refresh = page.get_by_role("button", name="Refresh")
        expect(page.get_by_text("Your household has no departure receipt yet.")).to_be_visible()
        expect(page.locator(".energy-total").filter(has_text="8.00 kWh")).to_be_visible()
        measure = page.get_by_role("radio", name="Grid import")
        measure.focus()
        page.keyboard.press("ArrowRight")
        expect(page.get_by_role("radio", name="Grid export")).to_be_checked()
        expect(page.locator(".energy-total")).to_have_text("1.00 kWh")
        page.keyboard.press("ArrowLeft")
        inspect_day = page.get_by_role("slider", name="Inspect energy by day")
        inspect_day.focus()
        page.keyboard.press("ArrowLeft")
        expect(inspect_day).to_have_attribute("aria-valuetext", "13 Jan 2026, Grid import, No report")
        page.get_by_role("button", name="Previous day").click()
        expect(inspect_day).to_have_attribute("aria-valuetext", "12 Jan 2026, Grid import, 0.00 kWh")
        page.get_by_label("History period").select_option("30")
        expect(page.locator(".energy-total")).to_have_text("11.00 kWh")
        expect(page.get_by_text("Grid import · 3 of 30 days reported.")).to_be_visible()
        send("home", 6000, "--missing-close")
        refresh.press("Enter")
        expect(page.get_by_role("heading", name="Departure unverified")).to_be_visible()
        page.get_by_text("Outlet meter readings", exact=True).focus()
        page.keyboard.press("Enter")
        expect(page.get_by_text("Missing", exact=True)).to_be_visible()
        send("home", 6000)
        refresh.click()
        expect(page.get_by_role("heading", name="1.25 kWh short")).to_be_visible()
        expect(page.get_by_text("4.75 of 6.00 kWh delivered to the outlet.")).to_be_visible()
        download = page.get_by_role("button", name="Download receipt JSON")
        with page.expect_download() as saved:
            download.click()
        data = json.loads(saved.value.path().read_text())
        assert (data["site"], data["shortfall_wh"]) == ("home", 1250)
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        send("second", 3600)
        page.set_extra_http_headers({"Authorization": "Bearer " + tokens["second"]})
        page.reload()
        expect(page.locator(".energy-total").filter(has_text="42.00 kWh")).to_be_visible()
        expect(page.locator(".energy-total").filter(has_text="8.00 kWh")).to_have_count(0)
        expect(page.get_by_role("heading", name="Charging goal met")).to_be_visible()

        # Reload the collector with this customer's grant revoked; keep the browser open.
        collector.terminate()
        collector.wait(timeout=10)
        config.write_text(config.read_text().replace(tokens["second"], secrets.token_hex(20)))
        wait_ready(collect, stack.enter_context(daemon(obsd, config)))
        refused_downloads = []
        page.on("download", lambda file: refused_downloads.append(file))
        download.click()
        expect(page.get_by_role("status")).to_contain_text("access has been withdrawn")
        expect(download).to_have_count(0)
        response = page.request.get(base + "/v1/me/departure", headers={
            "Authorization": "Bearer " + tokens["second"]})
        assert response.status == 401 and "no-store" in response.headers["cache-control"]
        assert not refused_downloads
        assert page.request.get(collect + "/portal").status == 404
        print("Verified customer isolation, three outlet outcomes, exact export and revocation.")


if __name__ == "__main__":
    run(*sys.argv[1:])
