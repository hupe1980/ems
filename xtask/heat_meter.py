# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Read the libmbus MULTICAL 601 capture through the gateway's actual serial port."""
import datetime
import errno
import os
import pty
import secrets
import select
import subprocess
import sys
import tempfile
import threading
import time
import tty
from pathlib import Path

from doctor import ports, running
from playwright.sync_api import Error as PlaywrightError
from playwright.sync_api import expect, sync_playwright

ROOT = Path(__file__).resolve().parents[1]
CAPTURE = bytes.fromhex((ROOT / "crates/ems-drv/tests/fixtures/mbus/kamstrup-601.hex").read_text())


def peer(master, stopped, mode, errors):
    reset = bytes.fromhex("10 40 11 51 16")
    read = bytes.fromhex("10 7b 11 8c 16")
    pending = bytearray()
    try:
        while not stopped.is_set():
            if not select.select([master], [], [], 0.1)[0]:
                continue
            try:
                chunk = os.read(master, 256)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                stopped.wait(0.05)
                continue
            if not chunk:
                stopped.wait(0.05)
                continue
            pending.extend(chunk)
            while len(pending) >= 5:
                request = bytes(pending[:5])
                del pending[:5]
                assert request in (reset, read), request.hex()
                status = mode[0]
                if status == "silent":
                    continue
                frame = bytearray(CAPTURE)
                if status == "wrong":
                    frame[7] ^= 1
                if status == "wrong_address":
                    frame[5] = 18
                if status == "continued":
                    # A valid current Wh register followed by the next-telegram marker.
                    frame[19:-2] = bytes([1, 3, 42, 0x1f])
                    frame[1] = frame[2] = len(frame) - 6
                if status == "large":
                    # Eight-byte integer and Wh VIF instead of the captured four-byte kWh.
                    frame[25:31] = bytes([7, 3]) + (2**53 + 1).to_bytes(8, "little")
                    frame[1] = frame[2] = len(frame) - 6
                if status == "zero":
                    frame[27:31] = bytes(4)
                frame[-2] = sum(frame[4:-2]) % 256
                if status == "corrupt":
                    frame[-2] ^= 1
                if request == reset:
                    os.write(master, request + b"\xe5")
                else:
                    os.write(master, request + frame[:11])
                    time.sleep(0.02)
                    os.write(master, frame[11:] + (frame if status == "collision" else b""))
    except Exception as error:
        errors.append(error)


def rehearse(binary, directory, path, mode):
    # SR-167, SR-168, SR-169. Exercise wire qualification and loss through the live gateway.
    port, grid_port = ports(2)
    base = f"http://127.0.0.1:{port}"
    token = secrets.token_hex(32)
    env = dict(os.environ, HEMS_TOKEN=token)
    config = directory / "house.toml"
    config.write_text(f'''mode = "shadow"
external_limit_controller = "Trial grid controller"
store_path = "box.redb"
[[drivers]]
kind = "sunspec"
asset = "netzanschluss-zaehler"
address = "127.0.0.1:{grid_port}"
unit = 1
listens_only = true
[service]
listen = "127.0.0.1:{port}"
[api]
token = "env:HEMS_TOKEN"
tls = false
[heat_meter]
path = "{path}"
address = 17
manufacturer = "KAM"
meter_id = "06855817"
poll_s = 2
''')
    with (directory / "gateway.log").open("w") as log, running(
        [binary, "run", "--config", str(config)], directory, env, log
    ) as gateway, sync_playwright() as playwright:
        chrome = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        browser = playwright.chromium.launch(executable_path=os.getenv("CHROME") or (str(chrome) if chrome.exists() else None))
        page = browser.new_page(viewport={"width": 320, "height": 740})
        for _ in range(100):
            if gateway.poll() is not None:
                raise RuntimeError((directory / "gateway.log").read_text().replace(token, "[redacted]"))
            try:
                page.goto(base + "/?lang=en#setup", timeout=2000)
                break
            except PlaywrightError:
                time.sleep(0.1)
        assert page.request.get(base + "/v1/trust").status == 401
        page.locator("#tok").fill(token)
        page.locator("#signin button").press("Enter")
        card = page.get_by_role("region", name="Heat meter")
        expect(card).to_contain_text("37,351,000 Wh", timeout=20000)
        expect(card).to_contain_text("KAM 06855817")
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        # Live diagnostics keep updating while an installer edits another Setup form.
        hosts = page.locator("#hosts")
        hosts.fill("192.0.2.4")
        mode[0] = "large"
        expect(card).to_contain_text("9,007,199,254,740,993 Wh", timeout=18000)
        last_good = None
        for fault, label, state in [
            ("collision", "Unusable reading", "invalid_reply"),
            ("wrong_address", "Unexpected meter", "wrong_meter"),
            ("corrupt", "Unusable reading", "invalid_reply"),
            ("wrong", "Unexpected meter", "wrong_meter"),
            ("continued", "Unusable reading", "invalid_reply"),
            ("silent", "Reading unavailable", "unavailable"),
        ]:
            mode[0] = fault
            expect(card).to_contain_text(label, timeout=18000)
            reading = page.request.get(base + "/v1/trust").json()["heat_meter"]
            assert reading["state"] == state and reading["delivered_heat_wh"] is None
            if last_good is None:
                last_good = reading["last_read_at"]
            assert last_good and reading["last_read_at"] == last_good
            assert "9,007,199,254,740,993" not in card.inner_text()
        expect(hosts).to_be_focused()
        expect(hosts).to_have_value("192.0.2.4")
        doctor = [binary, "doctor", "--config", str(config)]
        lost = subprocess.run(doctor, cwd=directory, env=env, capture_output=True, text=True, timeout=15)
        assert lost.returncode and "unavailable" in lost.stdout, lost.stdout
        received = datetime.datetime.fromtimestamp(last_good, datetime.UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        assert f"last heat reading  {received}" in lost.stdout
        mode[0] = "zero"
        expect(card).to_contain_text("0 Wh", timeout=18000)
        zero = page.request.get(base + "/v1/trust").json()["heat_meter"]
        assert zero["delivered_heat_wh"] == "0" and zero["state"] == "live"
        live = subprocess.run(doctor, cwd=directory, env=env, capture_output=True, text=True, timeout=15)
        assert "0 Wh delivered" in live.stdout, live.stdout
        mode[0] = "live"
        page.set_viewport_size({"width": 1280, "height": 800})
        page.locator("#lang").click()
        expect(page.get_by_role("region", name="Wärmezähler")).to_contain_text("37.351.000 Wh", timeout=18000)
        browser.close()


def main():
    master, slave = pty.openpty()
    tty.setraw(slave)
    path = os.ttyname(slave)
    mode, errors, stopped = ["live"], [], threading.Event()
    thread = threading.Thread(target=peer, args=(master, stopped, mode, errors))
    thread.start()
    try:
        with tempfile.TemporaryDirectory() as directory:
            rehearse(str(Path(sys.argv[1]).resolve()), Path(directory), path, mode)
    finally:
        stopped.set()
        thread.join()
        os.close(master)
        os.close(slave)
    if errors:
        raise errors[0]
    print("M-Bus heat meter verified through PTY, authenticated diagnostics, Setup and doctor.")


if __name__ == "__main__":
    main()
