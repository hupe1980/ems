# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Exercise the built challenge under a Pages-style prefix with a real Wasm response."""

import os
import subprocess
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from tempfile import TemporaryDirectory
from threading import Thread

from playwright.sync_api import expect, sync_playwright


class StaticFiles(SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def verify(browser, base):
    page = browser.new_page(viewport={"width": 320, "height": 760}, color_scheme="dark")
    requests, errors = [], []
    page.on("request", lambda request: requests.append(request.url))
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(base + "/")
    assert not any(".wasm" in url for url in requests), "landing page fetched Wasm"
    with page.expect_response(lambda response: ".wasm" in response.url) as module:
        page.locator('main a[href$="/peak-challenge/"]').first.click()
    assert module.value.ok and module.value.headers["content-type"].startswith("application/wasm")
    early = page.get_by_role("button", name="Start now", exact=True)
    delayed = page.get_by_role("button", name="Wait two minutes", exact=True)
    reveal = page.get_by_role("button", name="Reveal the meter close", exact=True)
    result, meter = page.locator("#peak-result"), page.locator("#peak-meter")
    early.focus()
    page.keyboard.press("Enter")
    expect(early).to_have_attribute("aria-pressed", "true")
    expect(result).to_contain_text("1,100")
    expect(result).to_contain_text("10 kWh")
    expect(meter).to_be_hidden()
    page.keyboard.press("Tab")
    expect(delayed).to_be_focused()
    page.keyboard.press("Space")
    expect(delayed).to_have_attribute("aria-pressed", "true")
    expect(result).to_contain_text("1,116")
    expect(result).to_contain_text("6 kWh")
    page.keyboard.press("Tab")
    expect(reveal).to_be_focused()
    page.keyboard.press("Enter")
    expect(meter).to_contain_text("285 kWh")
    expect(meter).to_contain_text("1,140")
    expect(meter).to_be_visible()
    early.click()
    expect(meter).to_be_visible()
    expect(meter).to_contain_text("1,140")
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
    assert not errors, errors
    assert all(url.startswith(base + "/") for url in requests), requests
    page.close()

    for mode in ("offline-module", "invalid-reading", "no-script"):
        page = browser.new_page(java_script_enabled=mode != "no-script", viewport={"width": 320, "height": 760})
        if mode == "offline-module":
            page.route("**/*.wasm*", lambda route: route.abort())
        elif mode == "invalid-reading":
            # Keep the real module, but exercise its unavailable-reading contract.
            page.add_init_script("""const instantiate = WebAssembly.instantiate;
                WebAssembly.instantiate = async (...args) => {
                    const {instance} = await instantiate(...args);
                    return {instance: {exports: {...instance.exports, projected_mean_kw: () => -1}}};
                };""")
        page.goto(base + "/peak-challenge/")
        example = page.locator("#peak-example")
        expect(example).to_be_visible()
        if mode == "no-script":
            example.locator("summary").click()
        else:
            expect(page.locator("#peak-unavailable")).to_be_visible()
        for value in ("1,100", "1,116", "1,140"):
            expect(example).to_contain_text(value)
        expect(page.locator("#peak-controls")).to_be_hidden()
        page.close()


def verify_solar(browser, base):
    page = browser.new_page(viewport={"width": 320, "height": 760}, color_scheme="light")
    page.goto(base + "/")
    page.get_by_role("link", name="Try solar EV charging").click()
    slider = page.get_by_role("slider", name="Surplus solar power")
    readings = page.locator(".solar-reading")
    expect(readings).to_have_text(["2.00 kW", "Paused"])
    slider.focus()
    page.keyboard.press("Home")
    expect(readings).to_have_text(["Paused", "Paused"])
    page.keyboard.press("End")
    expect(readings).to_have_text(["3.68 kW", "11.00 kW"])
    page.keyboard.press("ArrowLeft")
    expect(slider).to_have_attribute("aria-valuetext", "11.99 kW")
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
    page.close()
    for enabled in (True, False):
        page = browser.new_page(java_script_enabled=enabled, viewport={"width": 320, "height": 760})
        if enabled:
            page.route("**/*.wasm*", lambda route: route.abort())
        page.goto(base + "/solar-charging/")
        if enabled:
            expect(page.locator("#solar-failed")).to_be_visible()
        else:
            page.get_by_text("View the worked example", exact=True).click()
        expect(page.get_by_role("table")).to_be_visible()
        expect(page.locator("#solar-controls")).to_be_hidden()
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        page.close()


def main():
    chrome = os.environ.get("CHROME")
    mac = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    with TemporaryDirectory() as root, ThreadingHTTPServer(
        ("127.0.0.1", 0), partial(StaticFiles, directory=root)
    ) as server:
        base = f"http://127.0.0.1:{server.server_port}/ems"
        subprocess.run(["zola", "--root", "site", "build", "--base-url", base,
                        "--output-dir", str(Path(root, "ems"))], check=True)
        thread = Thread(target=server.serve_forever)
        thread.start()
        try:
            with sync_playwright() as pw:
                browser = pw.chromium.launch(executable_path=chrome or (str(mac) if mac.exists() else None))
                verify(browser, base)
                verify_solar(browser, base)
                browser.close()
        finally:
            server.shutdown()
            thread.join()
    print("Household solar charging and industrial peak challenge passed keyboard and fallback checks.")


if __name__ == "__main__":
    main()
