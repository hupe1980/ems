# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Commission a simulated RS-485 meter, verify identity rollback and observe a lost reading."""

import os
import re
import select
import ssl
import subprocess
import sys
import tempfile
import tomllib
from contextlib import contextmanager
from pathlib import Path

from doctor import ports, running, wait_for
from playwright.sync_api import expect, sync_playwright


def line(stream):
    if not select.select([stream], [], [], 15)[0]:
        raise TimeoutError("The RTU peer did not reply")
    return stream.readline().rstrip()


@contextmanager
def meter_peer(binary):
    peer = subprocess.Popen([binary], text=True, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    try:
        path = Path(line(peer.stdout))
        assert path.is_char_device(), path

        def command(action):
            peer.stdin.write(action + "\n")
            peer.stdin.flush()
            assert line(peer.stdout) == action

        yield path, command
    finally:
        peer.terminate()
        peer.wait(timeout=10)
        peer.stdin.close()
        peer.stdout.close()


def review(page, serial):
    form = page.locator("#meter")
    form.locator("[name=role][value=installer]").check()
    form.locator("[name=phases]").check()
    for key, value in {"fuse": "35", "energy": "20", "network": "7", "standing": "90"}.items():
        form.locator(f"[name={key}]").fill(value)
    form.locator("[name=link]").select_option("serial")
    expect(form.locator("[name=bus]")).to_have_value("panel")
    form.get_by_role("button", name="Review devices").press("Enter")
    expect(page.locator("#device")).to_contain_text(serial)
    expect(page.locator("#device")).to_be_focused()
    expect(page.locator("#block")).to_contain_text("serial-sunspec")
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")


def commission(page, base, token, path, command):
    page.goto(base + "/?lang=en")
    page.locator("#login [name=token]").fill(token)
    page.get_by_role("button", name="Sign in").click()
    expect(page.locator("#meter")).to_be_visible()
    assert page.request.get(base + "/v1/commissioning/serial-buses").json() == ["panel"]
    endpoint = base + "/v1/commissioning/first-meter"
    valid = dict(serial_bus="panel", unit=1, phases=3, fuse_a=35, energy_ct=20, network_ct=7, standing_eur=90, support="")
    for change in [{"serial_bus": "undeclared"}, {"unit": 0}, {"unit": 248}]:
        assert page.request.post(endpoint, data=valid | change).status == 400
    review(page, "grid-001")

    original = path.read_bytes()
    command("swap")
    page.locator("#apply").press("Enter")
    expect(page.locator("#status")).to_contain_text("configuration restored", timeout=100_000)
    assert path.read_bytes() == original

    review(page, "grid-002")
    page.locator("#apply").press("Enter")
    meter = page.locator("section.slot").filter(has_text="panel · Unit 1 · SunSpec 203")
    expect(meter).to_contain_text("grid-002", timeout=140_000)
    expect(meter).to_contain_text("Live")
    expect(meter).to_contain_text(re.compile(r"-375 W.*Import 45(?:\.0+)? kWh.*Export 12(?:\.0+)? kWh"))
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
    saved = tomllib.loads(path.read_text())
    assert saved["drivers"][0].items() >= {"kind": "serial-sunspec", "bus": "panel", "unit": 1}.items()


def observe_loss(page, command):
    meter = page.locator("section.slot").filter(has_text="panel · Unit 1 · SunSpec 203")
    neighbour = page.locator("section.slot").filter(has_text="panel · Unit 2 · SunSpec 203")
    expect(meter).to_contain_text("Live", timeout=60_000)
    expect(neighbour).to_contain_text("feeder-001", timeout=60_000)
    command("silent")
    expect(meter).to_contain_text("Reading unavailable", timeout=90_000)
    expect(meter).to_contain_text("Last reading")
    expect(meter).not_to_contain_text("Active power")
    expect(neighbour).to_contain_text("Live")
    expect(neighbour).to_contain_text(re.compile(r"900 W.*Export 0(?:\.0+)? kWh"))
    command("online")
    expect(meter).to_contain_text("Live", timeout=90_000)
    page.set_viewport_size({"width": 1280, "height": 800})
    expect(meter).to_contain_text("grid-002")
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")


def main():
    binary, fixture = (str(Path(arg).resolve()) for arg in sys.argv[1:])
    with tempfile.TemporaryDirectory() as directory, meter_peer(fixture) as (tty, command):
        path = Path(directory) / "gateway.toml"
        path.write_text(f'[serial_buses.panel]\npath = "{tty}"\nbaud = 19200\nparity = "none"\nstop_bits = 2\n')
        with path.open("a") as provisioned:
            provisioned.write('[heat_meter]\npath = "/dev/ems-trial-absent"\naddress = 17\nmanufacturer = "KAM"\nmeter_id = "06855817"\n')
        expected_heat = tomllib.loads(path.read_text())["heat_meter"]
        port, = ports(1)
        env = dict(os.environ, EMS_HEMSD_LISTEN=f"127.0.0.1:{port}")
        log = Path(directory) / "gateway.log"
        args = [binary, "run", "--config", str(path)]
        base = f"https://127.0.0.1:{port}"
        with sync_playwright() as playwright, log.open("w") as stream:
            chrome = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
            browser = playwright.chromium.launch(executable_path=os.getenv("CHROME") or (str(chrome) if chrome.exists() else None))
            page = browser.new_page(viewport={"width": 320, "height": 740}, locale="en-US", ignore_https_errors=True)
            with running(args, directory, env, stream) as gateway:
                wait_for(lambda: ssl.get_server_certificate(("127.0.0.1", port), timeout=2), gateway)
                token = wait_for(lambda: re.search(r"API  ([a-f0-9]{64})", log.read_text()), gateway)[1]
                commission(page, base, token, path, command)
                retained = tomllib.loads(path.read_text())["heat_meter"]
                assert expected_heat.items() <= retained.items()
                expect(page.get_by_role("region", name="Heat meter")).to_contain_text("KAM 06855817")
            with path.open("a") as config:
                config.write('\n[[drivers]]\nkind = "serial-sunspec"\nasset = "haushalt"\nbus = "panel"\nunit = 2\n')
            with running(args, directory, env, stream) as gateway:
                wait_for(lambda: ssl.get_server_certificate(("127.0.0.1", port), timeout=2), gateway)
                page.context.clear_cookies()
                page.goto(base + "/?lang=en#setup")
                page.locator("#tok").fill(token)
                page.locator("#signin button").press("Enter")
                expect(page.locator('[data-v="setup"]')).to_have_attribute("aria-current", "page")
                page.locator('[data-v="day"]').press("Enter")
                expect(page.locator('[data-v="day"]')).to_have_attribute("aria-current", "page")
                page.go_back()
                expect(page.locator('[data-v="setup"]')).to_have_attribute("aria-current", "page")
                observe_loss(page, command)
            browser.close()
    print("Serial commissioning, identity rollback and live/stale/recovered readings passed.")


if __name__ == "__main__":
    main()
