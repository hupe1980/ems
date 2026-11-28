# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.65.2"]
# ///
"""Exercise the browser ingress with disposable Dex, OAuth2 Proxy and NGINX."""

import base64
import os
import secrets
import subprocess
import time
from contextlib import ExitStack
from pathlib import Path
from tempfile import TemporaryDirectory
from urllib.request import urlopen

from playwright.sync_api import expect, sync_playwright

from customer_portal import port


def run():
    with ExitStack() as stack:
        directory = Path(stack.enter_context(TemporaryDirectory()))
        dexport, front = port(), port()
        issuer = f"http://127.0.0.1:{dexport}"
        public = f"http://127.0.0.1:{front}"
        name = "ems-oidc-" + secrets.token_hex(4)
        passwd = secrets.token_urlsafe(10)
        hashed = (
            subprocess.check_output(
                ["htpasswd", "-niB", "-C", "10", "test"], input=passwd + "\n", text=True
            )
            .strip()
            .split(":", 1)[1]
        )
        # Dex encodes the local user ID and connector ID in its subject.
        subject = (
            base64.urlsafe_b64encode(b"\x0a\x04home\x12\x05local").decode().rstrip("=")
        )
        (directory / "ems-household.map").write_text(
            "~^" + subject + '$ "Bearer trial-home";\n'
        )
        (directory / "ems-operator.map").write_text(
            '~^operator-test$ "Bearer trial-operator";\n'
        )
        (directory / "dex.yaml").write_text(f'''issuer: {issuer}
storage: {{type: memory}}
web: {{http: "0.0.0.0:{dexport}"}}
oauth2: {{skipApprovalScreen: true}}
staticClients:
- id: ems
  name: EMS
  secret: trial
  redirectURIs: ["{public}/oauth2/callback"]
enablePasswordDB: true
staticPasswords:
- email: home@example.com
  hash: "{hashed}"
  username: home
  userID: home
    ''')
        config = (
            Path("deploy/oidc/oauth2-proxy.cfg")
            .read_text()
            .replace("127.0.0.1:4180", f"0.0.0.0:{front}")
            .replace("__Host-ems", "ems-trial")
            .replace("cookie_secure = true", "cookie_secure = false")
        )
        (directory / "proxy.cfg").write_text(config)
        (directory / "gate.conf").write_text(
            Path("deploy/oidc/ems.conf").read_text()
            + """
    server {
        listen 127.0.0.1:7580;
        location = /v1/desk/energy {
            if ($http_authorization != "Bearer trial-operator") { return 401; }
            if ($http_cookie != "") { return 403; }
            default_type text/plain;
            return 200 "Scoped historian read";
        }
    }
    server {
        listen 127.0.0.1:7680;
        location / {
            if ($http_authorization != "Bearer trial-home") { return 401; }
            default_type text/plain;
            return 200 "Household overview";
        }
    }
    """
        )

        def container(label, params):
            args = ["docker", "run", "-d", "--name", name + label, *params]
            subprocess.run(args, check=True, stdout=subprocess.DEVNULL)
            stack.callback(
                subprocess.run,
                ["docker", "rm", "-f", name + label],
                stdout=subprocess.DEVNULL,
            )

        container(
            "gate",
            [
                "-p",
                f"127.0.0.1:{dexport}:{dexport}",
                "-p",
                f"127.0.0.1:{front}:{front}",
                "-v",
                f"{directory}/gate.conf:/etc/nginx/conf.d/default.conf:ro",
                "-v",
                f"{directory}:/run/secrets:ro",
                "nginx@sha256:8eb2b5540eae779603cc3db84b3986a15ecb5308b54303146eee9837c137b51a",
            ],
        )
        share = ["--network", "container:" + name + "gate"]
        container(
            "dex",
            [
                *share,
                "-v",
                f"{directory}/dex.yaml:/etc/dex/cfg:ro",
                "ghcr.io/dexidp/dex@sha256:5d0656fce7d453c0e3b2706abf40c0d0ce5b371fb0b73b3cf714d05f35fa5f86",
                "dex",
                "serve",
                "/etc/dex/cfg",
            ],
        )
        for _ in range(50):
            try:
                urlopen(issuer + "/.well-known/openid-configuration").close()
                break
            except OSError:
                time.sleep(0.2)
        container(
            "proxy",
            [
                *share,
                "-v",
                f"{directory}/proxy.cfg:/etc/ems.cfg:ro",
                "quay.io/oauth2-proxy/oauth2-proxy@sha256:d963570b1f7e399bc42caace45433aab09e7ae76de5610fde97ba32029b6150a",
                "--config=/etc/ems.cfg",
                "--client-id=ems",
                "--client-secret=trial",
                "--oidc-issuer-url=" + issuer,
                "--redirect-url=" + public + "/oauth2/callback",
                "--cookie-secret=" + secrets.token_hex(16),
            ],
        )
        for _ in range(50):
            try:
                urlopen(public + "/ping").close()
                break
            except OSError:
                time.sleep(0.2)
        else:
            subprocess.run(["docker", "logs", name + "proxy"])
            raise RuntimeError("no proxy")
        local = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        chrome = os.environ.get("CHROME") or (str(local) if local.exists() else None)
        browser = stack.enter_context(sync_playwright()).chromium.launch(
            executable_path=chrome
        )
        stack.callback(browser.close)
        page = browser.new_page()
        page.goto(public + "/portal")
        page.locator("input[type=email],input[name=login]").first.fill(
            "home@example.com"
        )
        page.locator("input[type=password]").fill(passwd)
        page.locator("button[type=submit]").click()
        page.get_by_role("button", name="Grant Access").click()
        expect(page.locator("body")).to_contain_text(
            "Household overview", timeout=10000
        )
        page.set_extra_http_headers(
            {
                "X-Forwarded-User": "operator-test",
                "Authorization": "Bearer trial-operator",
            }
        )
        assert page.goto(public + "/desk").status == 403
        assert page.goto(public + "/v1/desk/canary").status == 403
        assert page.goto(public + "/v1/desk/energy?site=haus&day=2026-01-15").status == 403
        page.set_extra_http_headers({})
        # Grant this signed-in subject access and exercise the production route.
        (directory / "ems-operator.map").write_text(
            "~^" + subject + '$ "Bearer trial-operator";\n'
        )
        subprocess.run(["docker", "exec", name + "gate", "nginx", "-s", "reload"], check=True)
        for _ in range(30):
            if page.goto(public + "/v1/desk/energy?site=haus&day=2026-01-15").status == 200:
                break
            time.sleep(0.1)
        expect(page.locator("body")).to_have_text("Scoped historian read")
        assert page.goto(public + "/v1/enrol").status == 404
        page.goto(public + "/oauth2/sign_out")
        page.goto(public + "/portal")
        expect(page.locator("input[type=password]")).to_be_visible()
        print("OIDC code flow, authorized household, denied desk and sign-out passed")


if __name__ == "__main__":
    run()
