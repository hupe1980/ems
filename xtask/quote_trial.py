# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Measure real departure quotes from input release to the matching rendered price."""
import json
import math
import os
import platform
import statistics
import subprocess
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright


def measure(page, base):
    assert page.request.post(base + "/v1/session", data={"token": os.environ["HEMS_QUOTE_TRIAL_TOKEN"]}).ok
    page.goto(base + "/?lang=en#plan")
    card = page.locator('[data-goal][data-tank=""]')
    slider = card.locator('input[type="range"]')
    slider.wait_for(state="visible", timeout=15000)
    times = []
    for index in range(21):
        target = 60 + index
        with page.expect_response(lambda r: r.request.method == "POST" and r.url.endswith("/quote"), timeout=45000) as observed:
            slider.evaluate("""(slider, value) => {
                slider.value = value;
                performance.mark("goal-release");
                slider.dispatchEvent(new Event("change", {bubbles:true}));
            }""", str(target))
        response = observed.value
        assert response.ok, f"Quote {target}% refused with HTTP {response.status}"
        assert math.isclose(response.request.post_data_json["target_soc"], target / 100)
        euros = response.json()["extra_eur"]
        assert euros is not None, f"Quote {target}% missing price"
        duration = page.evaluate("""async ([target, euros]) => {
            const until = performance.now() + 4000;
            const price = Math.abs(euros) < 0.005 ? t("qFree") :
                t(euros > 0 ? "qMore" : "qLess", {eur:eur(Math.abs(euros))});
            while (performance.now() < until) {
                await new Promise(requestAnimationFrame);
                const q = document.querySelector('[data-goal][data-tank=""] [data-quote]');
                if (q?.checkVisibility() && q.textContent.includes(price) &&
                    q.querySelector("button")?.textContent.includes(target + " %")) {
                    return performance.measure("quote-visible", "goal-release").duration;
                }
            }
            throw Error("Missing matching rendered price for " + target);
        }""", [target, euros])
        if index:
            times.append(round(duration, 1))
        page.wait_for_timeout(1050)
    return times


if __name__ == "__main__":
    base = sys.argv[1]
    mac_chrome = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    with sync_playwright() as pw:
        browser = pw.chromium.launch(executable_path=os.getenv("CHROME") or (str(mac_chrome) if mac_chrome.exists() else None))
        ms = measure(browser.new_page(viewport={"width": 320, "height": 720}), base)
        percentile = sorted(ms)[18]
        cpu = platform.processor() or platform.machine()
        if platform.system() == "Darwin":
            cpu = subprocess.check_output(["sysctl", "-n", "machdep.cpu.brand_string"], text=True).strip()
        print(json.dumps({"revision": subprocess.check_output(["git","rev-parse","--short","HEAD"],text=True).strip(),
                          "runner": platform.platform(), "cpu": cpu, "browser": browser.version,
                          "solver": "HiGHS, 96 slots, 20s solver budget", "accepted": len(ms), "refusals": 0,
                          "median_ms": statistics.median(ms), "p95_ms": percentile, "samples_ms": ms}))
        assert percentile < 2000, f"Quote p95 exceeded the two-second budget: {percentile} ms"
