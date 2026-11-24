# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Exercise the shipped service worker with the decrypted loopback reduction."""
import json
import os
import subprocess
import time
from functools import partial
from http.server import ThreadingHTTPServer
from pathlib import Path
from tempfile import TemporaryDirectory
from threading import Thread

from playwright.sync_api import sync_playwright

from history_browser import Assets


def exercise(browser, base, documents):
    context = browser.new_context(permissions=["notifications"])
    page = context.new_page()
    page.goto(base + "/icon.svg")
    with context.expect_event("serviceworker") as installed:
        page.evaluate("navigator.serviceWorker.register('/sw.js')")
    worker = installed.value
    page.evaluate("navigator.serviceWorker.ready")
    for document in documents:
        expected = document["notification"]
        # Deliver the decrypted payload at the worker boundary and await the
        # real showNotification operation before inspecting the browser store.
        worker.evaluate("""async document => {
            const push = new PushEvent("push", {data: JSON.stringify(document)});
            let shown;
            push.waitUntil = promise => { shown = promise };
            self.dispatchEvent(push);
            if (!shown) throw Error("Worker did not handle the push");
            await shown;
        }""", document)
        # Await each browser query before testing its result. A pending Promise
        # is truthy even when getNotifications eventually returns an empty list.
        deadline = time.monotonic() + 5
        while True:
            actual = page.evaluate("""async tag => {
                const registration = await navigator.serviceWorker.ready;
                const [n] = await registration.getNotifications({tag});
                return n && {title: n.title, body: n.body, data: n.data, lang: n.lang};
            }""", expected["tag"])
            if actual and actual.get("data", {}).get("event_id") == expected["data"]["event_id"]:
                break
            if time.monotonic() >= deadline:
                raise AssertionError(f"Notification not displayed: expected {expected!r}, observed {actual!r}")
            time.sleep(0.05)
        wanted = {k: expected[k] for k in ("title", "body", "lang")} | {
            "data": {**expected["data"], "navigate": expected["navigate"]}
        }
        assert actual == wanted, (actual, wanted)
        worker.evaluate("""async tag => {
            const [notification] = await self.registration.getNotifications({tag});
            const event = new Event("notificationclick");
            let clicked;
            event.notification = notification;
            event.waitUntil = promise => { clicked = promise };
            // A synthetic click lacks the OS activation needed to focus a window.
            // Keep the real window lookup, navigation and notification close.
            const focus = WindowClient.prototype.focus;
            WindowClient.prototype.focus = async function () { return this };
            try { self.dispatchEvent(event); await clicked }
            finally { WindowClient.prototype.focus = focus }
        }""", expected["tag"])
        page.wait_for_url(base + "/#now", timeout=5000)
        assert worker.evaluate("(async () => (await self.registration.getNotifications()).length)()") == 0
        page.goto(base + "/icon.svg")
    context.close()


def main():
    ui = Path(__file__).resolve().parents[1] / "services/hemsd/ui"
    with TemporaryDirectory() as tmp, ThreadingHTTPServer(("127.0.0.1", 0), partial(Assets, directory=str(ui))) as server:
        Thread(target=server.serve_forever, daemon=True).start()
        try:
            base = f"http://127.0.0.1:{server.server_port}"
            capture = Path(tmp) / "decrypted.json"
            subprocess.run(["cargo", "test", "-p", "hemsd", "--lib",
                            "a_reduction_is_exactly_two_encrypted_notifications_only_the_browser_can_read"],
                           env={**os.environ, "EMS_TEST_NOTIFICATION_ORIGIN": base,
                                "EMS_TEST_NOTIFICATION_CAPTURE": str(capture)}, check=True)
            documents = json.loads(capture.read_text())
            assert len(documents) == 2
            with sync_playwright() as pw:
                local = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
                executable = os.getenv("CHROME") or (str(local) if local.exists() else None)
                browser = pw.chromium.launch(executable_path=executable)
                exercise(browser, base, documents)
                browser.close()
        finally:
            server.shutdown()
    print("Verified both encrypted reduction messages, displayed worker notifications and tap destinations.")


if __name__ == "__main__":
    main()
