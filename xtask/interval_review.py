# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Check the offline industrial review against real CLI output at mobile width."""

import json
import os
import subprocess
import sys
from copy import deepcopy
from datetime import datetime
from pathlib import Path
from tempfile import TemporaryDirectory
from zoneinfo import ZoneInfo

from playwright.sync_api import expect, sync_playwright

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "crates/ems-sim/fixtures/shift-trace.json"
CLI = ROOT / "target/debug/cemsd"
LABEL = {
    "register_decreased": "Register decreased",
    "missing_boundary": "Missing register boundary",
    "missing_closed_interval": "Missing independent close",
    "outside_tolerance": "Outside tolerance",
    "within_tolerance": "Within tolerance",
}


def cases():
    reference = json.loads(FIXTURE.read_text())
    yield reference, ["within_tolerance"] * 2
    missing = deepcopy(reference)
    missing["register"].pop(3)
    yield missing, ["missing_boundary"] * 2
    reset = deepcopy(reference)
    reset["register"][1]["wh"] = 0
    reset["closed"].clear()
    yield reset, ["register_decreased", "missing_closed_interval"]
    mismatch = deepcopy(reference)
    mismatch["closed"][0]["wh"] += 2
    mismatch["closed"][1]["wh"] -= 1
    yield mismatch, ["outside_tolerance"] * 2
    yield {
        "start": "9999-12-31T23:00:00Z", "end": "9999-12-31T23:15:00Z",
        "register": [], "closed": [], "tolerance_wh": 0,
    }, ["missing_boundary"]
    zero = deepcopy(reference)
    for point in zero["register"] + zero["closed"]:
        point["wh"] = 0
    zero["closed"][1]["wh"] = 2**64 - 1
    yield zero, ["within_tolerance", "outside_tolerance"]


def run(source, *args):
    return subprocess.run([CLI, "reconcile", *args, source], capture_output=True)


def review(page, directory, trace, statuses):
    source = directory / "meter.json"
    source.write_text(json.dumps(trace))
    data = run(source)
    html = run(source, "--format", "html")
    assert data.returncode == html.returncode == int(any(s != "within_tolerance" for s in statuses))
    report = json.loads(data.stdout)
    assert [row["status"] for row in report["intervals"]] == statuses
    assert directory.name.encode() not in html.stdout
    path = directory / "review.html"
    path.write_bytes(html.stdout)
    page.goto(path.as_uri())
    assert page.locator("script, [src], link").count() == 0
    assert all(a.startswith("#q") for a in page.locator("a").evaluate_all("(links) => links.map(a => a.getAttribute('href'))"))
    noun = "quarter" if len(statuses) == 1 else "quarters"
    expect(page.locator(".summary")).to_have_text(f"{report['matching_intervals']} of {len(statuses)} {noun} within tolerance")
    expect(page.locator("header")).to_contain_text(f"{report['compared_intervals']} compared · Tolerance {trace['tolerance_wh']} Wh")
    sections = page.locator("section")
    expect(sections).to_have_count(len(statuses))
    for index, row in enumerate(report["intervals"]):
        section = sections.nth(index)
        local = datetime.fromisoformat(row["start"])
        if 1900 <= local.year <= 9998:
            local = local.astimezone(ZoneInfo("Europe/Berlin"))
        offset = local.isoformat()[-6:]
        expect(section.locator("h2")).to_have_text(f"{local:%Y-%m-%d %H:%M} {offset} · {LABEL[row['status']]}")
        expect(section.locator("time")).to_have_attribute("datetime", row["start"])
        if row["status"] != "within_tolerance":
            expect(section).to_contain_text(f"Allowed difference · {trace['tolerance_wh']} Wh")
        values = [row[field] for field in ("register_wh", "closed_wh", "difference_wh")]
        expected = [("Unavailable" if value is None else str(value)) for value in values]
        if values[2] is not None and values[2] > 0:
            expected[2] = f"+{values[2]}"
        expect(section.locator("td")).to_have_text(expected)
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
    exceptions = [i for i, status in enumerate(statuses) if status != "within_tolerance"]
    expect(page.get_by_role("link")).to_have_count(len(exceptions))
    for order, index in enumerate(exceptions):
        page.keyboard.press("Tab")
        expect(page.locator(":focus")).to_have_text("Next exception" if order else "Go to first exception")
        page.keyboard.press("Enter")
        expect(page.locator(f"#q{index}")).to_be_focused()


def commission_review(page, directory):
    meter = ROOT / "services/cemsd/examples/opcua"
    lines = (meter / "readings.ndjson").read_bytes().splitlines()
    source = directory / "capture.ndjson"
    boundary = json.loads(lines[1])
    boundary["status"] = 0x40900000
    del boundary["value"]
    source.write_bytes(b"\n".join([lines[0], json.dumps(boundary).encode(), lines[2]]))
    outcome = subprocess.run([CLI, "commission-meter", "--contract", meter / "contract.json",
                              "--readings", source, "--window", meter / "window.json",
                              "--format", "html"], capture_output=True)
    assert outcome.returncode == 1 and outcome.stdout
    path = directory / "review.html"
    path.write_bytes(outcome.stdout)
    page.goto(path.as_uri())
    expect(page.get_by_role("heading", name="Meter commissioning")).to_be_visible()
    expect(page.get_by_text("Capture line 2 · OPC UA status 0x40900000")).to_be_visible()
    if len(sys.argv) == 2:
        page.set_viewport_size({"width": 900, "height": 1350})
        page.goto(path.as_uri())
        page.screenshot(path=sys.argv[1])
        page.set_viewport_size({"width": 320, "height": 720})
    page.get_by_text("Register identity").focus()
    page.keyboard.press("Enter")
    expect(page.locator("dd").last).to_have_text("s=GridImportWh")
    page.get_by_role("link", name="Go to first exception").focus()
    page.keyboard.press("Enter")
    expect(page.locator("#q0")).to_be_focused()
    for label, target in [("Inspect 00:45 UTC quarter", "#q0"), ("Inspect 01:00 UTC quarter", "#q1")]:
        page.get_by_role("link", name=label).focus()
        page.keyboard.press("Enter")
        expect(page.locator(target)).to_be_focused()
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")


def main():
    default = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    with TemporaryDirectory(prefix="private-meter-") as temp, sync_playwright() as pw:
        directory = Path(temp).resolve()
        browser = pw.chromium.launch(executable_path=os.getenv("CHROME") or (str(default) if default.exists() else None))
        page = browser.new_page(java_script_enabled=False, offline=True, viewport={"width": 320, "height": 720})
        requests = []
        page.on("request", lambda req: requests.append(req.url))
        for trace, statuses in cases():
            review(page, directory, trace, statuses)
        commission_review(page, directory)
        assert set(requests) == {(directory / "review.html").as_uri()}, requests
        source = directory / "invalid.json"
        for content in (b"invalid", b" " * (4 * 1024 * 1024 + 1), json.dumps({
            "start": "9999-12-31T23:00:00-01:00", "end": "9999-12-31T23:15:00-01:00",
            "register": [], "closed": [], "tolerance_wh": 0,
        }).encode()):
            source.write_bytes(content)
            for args in ([], ["--format", "html"]):
                result = run(source, *args)
                assert result.returncode == 1 and not result.stdout and result.stderr
        browser.close()
    print("Offline interval review matches JSON for every status, signed Wh, clock offsets and keyboard jump.")


if __name__ == "__main__":
    main()
