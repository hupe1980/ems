"""Exercise household installation, recovery and first-run commissioning against the virtual house."""
import json
import os
import re
import secrets
import signal
import ssl
import subprocess
import sys
import tempfile
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

from doctor import ports, running, wait_for


def test(binary, directory):
    api, ship = ports(2)
    config = directory / "house.toml"
    token = secrets.token_hex(24)
    env = dict(os.environ, HEMS_TOKEN=token, EMS_REVIEW_TRIAL=token)
    with (directory / "trial.log").open("w") as log, running([
        binary, "house", "--listen", "127.0.0.1:1502", "--api", f"127.0.0.1:{api}",
        "--ship", f"127.0.0.1:{ship}", "--out", str(config),
    ], directory, env, log) as house:
        written = wait_for(config.read_text, house)
        # Keep only the grid driver configured. The simulated inverter still answers.
        site = re.sub(r'\[\[drivers\]\]\nkind = "sunspec".*?(?=\[\[drivers\]\]|\Z)', '', written, flags=re.S)
        original = 'store_path = "box.redb"\n[api]\ntls = true\ntoken = "env:EMS_REVIEW_TRIAL"\n' + site
        config.write_text(original)
        base = f"https://127.0.0.1:{api}"
        with running([binary, "run", "--config", str(config)], directory, env, log) as gateway:
            pem = directory / "gateway.pem"
            pem.write_text(wait_for(lambda: ssl.get_server_certificate(("127.0.0.1", api), timeout=2), gateway))
            env["GATEWAY_CERT"] = str(pem)
            trust = ssl.create_default_context(cafile=str(pem))

            def request(path, data=None):
                req = urllib.request.Request(f"https://localhost:{api}" + path, data=data, headers={
                    "authorization": "Bearer " + token, "content-type": "application/json"})
                with urllib.request.urlopen(req, timeout=10, context=trust) as response:
                    return response.read()

            def discover(unit):
                payload = json.dumps(dict(hosts=["127.0.0.1"], unit=unit, port=1502)).encode()
                reply = request("/v1/discover", payload)
                assert token.encode() not in reply and b"EMS_REVIEW_TRIAL" not in reply
                return json.loads(reply)["modbus"][0]

            first = discover(1)["review"]
            assert first["refusal"] is None and first["proposal"]
            # Out-of-band edits invalidate every proposal issued from the previous file.
            config.write_text(original + "# edited\n")
            try:
                request("/v1/commissioning/change", json.dumps({"id": first["proposal"]}).encode())
                raise AssertionError("A stale proposal changed the configuration")
            except urllib.error.HTTPError as error:
                assert error.code == 409
            config.write_text(original)
            assert "review" not in discover(2)
            subprocess.run(["node", str(Path(__file__).with_name("media.mjs")), "commissioning-review", base],
                           check=True, env=env, timeout=50)
            assert config.read_text() == original
            subprocess.run(["node", str(Path(__file__).with_name("media.mjs")), "price-sheet", base],
                           check=True, env=env, timeout=60)
            assert all(device["asset"] != "pv" for device in json.loads(request("/v1/trust"))["devices"])
            # The reviewed unit disappears between confirmation and supervised startup.
            restore_id = discover(1)["review"]["proposal"]
            house.send_signal(signal.SIGSTOP)
            try:
                request("/v1/commissioning/change", json.dumps({"id": restore_id}).encode())
                def restored():
                    try:
                        change = json.loads(request("/v1/commissioning/change"))
                        return change == {"id": restore_id, "outcome": "restored", "role":"pv"}
                    except (urllib.error.URLError, ConnectionError):
                        return False
                wait_for(restored, gateway)
            finally:
                house.send_signal(signal.SIGCONT)
            assert config.read_text() == original
            subprocess.run(["node", str(Path(__file__).with_name("media.mjs")), "commissioning-review", base],
                           check=True, env=dict(env, REVIEW_APPLY="1"), timeout=90)
            assert config.read_text() == original + "\n" + first["toml"]
            assert any(device["asset"] == "pv" and device["state"] == "responding"
                       for device in json.loads(request("/v1/trust"))["devices"])

        first_api, = ports(1)
        new_config = directory / "new.toml"
        first_env = dict(env, EMS_HEMSD_LISTEN=f"127.0.0.1:{first_api}")
        log_file = directory / "first-run.log"
        with log_file.open("w") as first_log, running([binary, "run", "--config", str(new_config)],
                                                     directory, first_env, first_log) as fresh:
            pem.write_text(wait_for(lambda: ssl.get_server_certificate(("127.0.0.1", first_api), timeout=2), fresh))
            match = wait_for(lambda: re.search(r"API  ([a-f0-9]{64})", log_file.read_text()), fresh)
            subprocess.run(["node", str(Path(__file__).with_name("media.mjs")), "first-run-install", f"https://127.0.0.1:{first_api}"],
                           check=True, env=dict(first_env, HEMS_TOKEN=match[1], GATEWAY_CERT=str(pem)), timeout=90)
            saved = tomllib.loads(new_config.read_text())
            assert saved["drivers"][0]["unit"] == 2 and saved["drivers"][0]["listens_only"]
            assert saved["site"]["pv_kwp"] == 8 and saved["site"]["battery_kwh"] == 10
            assert saved["mode"] == "shadow"
            by_asset = {d["asset"]: d for d in saved["drivers"]}
            assert by_asset["pv"]["rating_kw"] == 8
            assert by_asset["battery"]["listens_only"]
            assert {d["asset"] for d in saved["drivers"]} == {"netzanschluss-zaehler", "pv", "battery"}
        subprocess.run([binary, "run", "--config", str(new_config), "--check"], check=True, env=first_env, stdout=subprocess.DEVNULL)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as temporary:
        test(str(Path(sys.argv[1]).resolve()), Path(temporary))
