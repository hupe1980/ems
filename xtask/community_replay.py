# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Verify the local allocation replay from independent member and coordinator sessions."""

import os
import subprocess
import sys
from contextlib import contextmanager
from pathlib import Path
from urllib.parse import urljoin, urlsplit

from playwright.sync_api import expect, sync_playwright


@contextmanager
def community(binary, viewer):
    with subprocess.Popen(
        [binary, "community", "--viewer", viewer, "--listen", "127.0.0.1:0"],
        stderr=subprocess.PIPE, text=True,
    ) as process:
        try:
            login = process.stderr.readline().strip()
            address = process.stderr.readline().strip()
            if not (login.startswith("Local demo login: ") and address.startswith("Open http://127.0.0.1:")):
                raise RuntimeError(f"{viewer} did not announce its local demo")
            user, password = login.removeprefix("Local demo login: ").split(" / ")
            base = address.removeprefix("Open ").rstrip("/")
            yield base, {"username": user, "password": password}
        finally:
            process.terminate()


def activate(page, control, key="Enter"):
    for _ in range(24):
        if control.evaluate("el => el === document.activeElement"):
            href = control.get_attribute("href")
            target = urljoin(page.url, href) if href else None
            page.keyboard.press(key)
            if target:
                page.wait_for_url(target)
            return
        page.keyboard.press("Tab")
    raise AssertionError(f"Keyboard could not reach {control.inner_text()} at {page.url}")


def value(kwh, suffix, missing):
    return missing if kwh is None else f"{kwh} kWh {suffix}"


def compare(page, role):
    location = urlsplit(page.url)
    reply = page.request.get(f"{location.scheme}://{location.netloc}/v1/demo/community?{location.query}")
    assert reply.ok and reply.headers.get("cache-control") == "no-store"
    data = reply.json()
    expect(page.locator("#intervall")).to_have_text(data["period"])
    expect(page.get_by_text(data["status"], exact=True)).to_be_visible()
    for row in data["rows"]:
        heading = "Ihr Anteil" if role == "member-a" else f"Mitglied {row['member'].upper()}"
        section = page.get_by_role("region", name=heading)
        expect(section.locator("strong")).to_have_text(value(row["shared_kwh"], "zugeteilt", "Zuteilung ausstehend"))
        expect(section.locator("p")).to_have_text(value(row["consumed_kwh"], "verbraucht", "Verbrauchsmessung ausstehend"))
    balance = page.get_by_role("region", name="Bilanz")
    if role == "member-a":
        assert [row["member"] for row in data["rows"]] == ["a"] and "coordinator" not in data
        expect(balance).to_have_count(0)
        expect(page.get_by_role("heading", name="Mitglied B")).to_have_count(0)
    else:
        c = data["coordinator"]
        residual = "ausstehend" if c["residual_kwh"] is None else c["residual_kwh"] + " kWh"
        expect(balance.locator("strong")).to_have_text(residual)
        expect(balance.locator("p")).to_have_text(f"{c['measured']}/{c['expected']} Messungen · Erzeugung {c['supply_kwh']} kWh")
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"), page.url
    return data


def journey(page, base, role):
    page.goto(base + "/?interval=1&case=static")
    first = compare(page, role)
    assert first["period"].endswith("+02:00")
    quarters = page.get_by_role("navigation", name="Viertelstunde")
    activate(page, quarters.get_by_role("link", name="+01:00"))
    expect(page.locator("#intervall")).to_be_focused()
    second = compare(page, role)
    assert second["period"].endswith("+01:00") and second["rows"][0]["shared_kwh"] == "2"
    if role == "coordinator":
        assert second["coordinator"] == {"measured": 2, "expected": 2, "supply_kwh": "10", "residual_kwh": "3"}
    disclosure = page.get_by_text("Aufteilungsschlüssel", exact=True)
    activate(page, disclosure, "Space")
    expect(page.get_by_text("Statisch · gleiche Gewichte", exact=True)).to_be_visible()
    page.keyboard.press("Space")
    cases = page.get_by_role("navigation", name="Messfälle")
    activate(page, cases.get_by_role("link", name="Kaskadierend"))
    cascade = compare(page, role)
    assert cascade["key"] == "Kaskadierend"
    if role == "coordinator":
        assert cascade["coordinator"]["residual_kwh"] == "0"
    activate(page, cases.get_by_role("link", name="Messlücke"))
    missing = compare(page, role)
    assert all(r["shared_kwh"] is None for r in missing["rows"])
    if role == "coordinator":
        assert missing["coordinator"]["measured"] == 1 and missing["coordinator"]["residual_kwh"] is None
        activate(page, cases.get_by_role("link", name="Nachlieferung"))
        restored = compare(page, role)
        assert restored == second
        activate(page, cases.get_by_role("link", name="Nullverbrauch"))
        zero = compare(page, role)
        assert zero["rows"][1]["consumed_kwh"] == "0" and zero["coordinator"]["measured"] == 2
        page.goto(base + "/?case=outage&interval=2&member=a")
        previous = compare(page, role)["last_complete"]
        activate(page, page.get_by_role("link", name=previous["period"], exact=True).first)
        assert "member=a" in page.url
        assert compare(page, role) == previous
    else:
        page.go_back()
        assert compare(page, role) == cascade
        for route in ("/", "/v1/demo/community"):
            denied = page.request.get(base + route + "?member=b&interval=2")
            assert denied.status == 404 and not denied.body()
    return second


def run(binary):
    with sync_playwright() as pw:
        default = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        chrome = os.environ.get("CHROME") or (str(default) if default.exists() else None)
        browser = pw.chromium.launch(executable_path=chrome)
        credentials = []
        member_snapshot = None
        for role in ("member-a", "coordinator", "member-a"):
            with community(binary, role) as (base, auth):
                for prior in credentials:
                    old = pw.request.new_context(http_credentials=prior)
                    try:
                        for route in ("/", "/v1/demo/community"):
                            assert old.get(base + route).status == 401
                    finally:
                        old.dispose()
                context = browser.new_context(http_credentials=auth, viewport={"width": 320, "height": 720}, locale="de-DE")
                try:
                    page = context.new_page()
                    if role == "member-a" and member_snapshot is not None:
                        page.goto(base + "/?interval=2&case=static")
                        assert compare(page, role) == member_snapshot
                    else:
                        snapshot = journey(page, base, role)
                        if role == "member-a":
                            member_snapshot = snapshot
                finally:
                    context.close()
                credentials.append(auth)
        browser.close()


if __name__ == "__main__":
    run(sys.argv[1] if len(sys.argv) == 2 else "target/debug/bemsd")
    print("Community allocation passed for both roles, both UTC offsets and a fresh launch at 320 CSS pixels.")
