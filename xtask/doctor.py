"""Check household diagnostics against the virtual house over authenticated HTTPS."""
import json
import os
import re
import secrets
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import tomllib
from contextlib import ExitStack, contextmanager
from pathlib import Path


def ports(count):
    with ExitStack() as stack:
        sockets = [stack.enter_context(socket.socket()) for _ in range(count)]
        for sock in sockets:
            sock.bind(("127.0.0.1", 0))
        return [sock.getsockname()[1] for sock in sockets]


@contextmanager
def running(args, directory, env, log):
    process = subprocess.Popen(args, cwd=directory, env=env, stdout=log, stderr=log)
    try:
        yield process
    finally:
        process.terminate()
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def wait_for(read, process):
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline and process.poll() is None:
        try:
            value = read()
            if value:
                return value
        except (ConnectionRefusedError, FileNotFoundError, TimeoutError):
            pass
        time.sleep(0.25)
    raise RuntimeError("The virtual gateway did not reach the expected state")


def test(binary, directory):
    api, modbus, ship, disconnected = ports(4)
    config = directory / "house.toml"
    pem = directory / "gateway.pem"
    token = secrets.token_hex(24)
    env = dict(os.environ, HEMS_TOKEN=token, EMS_DOCTOR_TRIAL=token)
    doctor = [binary, "doctor", "--config", str(config)]

    def diagnose(trusted=True, credential=token):
        args = doctor + (["--ca-cert", str(pem)] if trusted else [])
        return subprocess.run(args, env=dict(env, HEMS_TOKEN=credential), cwd=directory,
                              capture_output=True, text=True, timeout=15)

    with (directory / "gateway.log").open("w") as log, running([
        binary, "house", "--listen", f"127.0.0.1:{modbus}", "--api", f"127.0.0.1:{api}",
        "--ship", f"127.0.0.1:{ship}", "--out", str(config),
    ], directory, env, log) as house:
        written = wait_for(config.read_text, house)
        base = 'store_path = "box.redb"\n[api]\ntls = true\ntoken = "env:EMS_DOCTOR_TRIAL"\n' + written
        ski, = tomllib.loads(base)["ship"]["trust"]
        base = re.sub(r'^trust = \[.*\]$', 'trust = []', base, count=1, flags=re.MULTILINE)
        fingerprints = None
        meter = next(d["asset"] for d in tomllib.loads(base)["drivers"] if d.get("unit") == 2)
        for fault in (False, True):
            candidate = base
            if fault:
                candidate, count = re.subn(r'address = "[^"]+"(\nunit = 2\n)',
                                           rf'address = "127.0.0.1:{disconnected}"\1', base)
                assert count == 1
            config.write_text(candidate)
            with running([binary, "run", "--config", str(config)], directory, env, log) as gateway:
                pem.write_text(wait_for(lambda: ssl.get_server_certificate(("127.0.0.1", api), timeout=2), gateway))

                started = time.monotonic()
                browser_env = dict(env, GATEWAY_CERT=str(pem), PAIR_SKI="" if fault else ski)
                script = Path(__file__).resolve().with_name("media.mjs")
                sheet = subprocess.run(["node", str(script), "handover-test", f"https://127.0.0.1:{api}"],
                                       env=browser_env, text=True, stdout=subprocess.PIPE,
                                       check=True, timeout=120)
                printed = json.loads(sheet.stdout)
                if fault:
                    assert printed == fingerprints, "The handover identity changed after restarting the gateway"
                else:
                    assert time.monotonic() - started < 120
                    fingerprints = printed

                def ready():
                    result = diagnose()
                    if fault:
                        return result.returncode != 0 and f"{meter}  silent" in result.stdout and "responding" in result.stdout
                    return result.returncode == 0 and "certificate verified" in result.stdout and "recent tick" in result.stdout

                wait_for(ready, gateway)
                if not fault:
                    assert diagnose(trusted=False).returncode != 0
                    refused = diagnose(credential=secrets.token_hex(24))
                    assert refused.returncode != 0 and "401" in refused.stderr
                    assert token not in refused.stdout + refused.stderr
    print("Doctor verified the gateway and identified its disconnected meter.")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as workspace:
        test(str(Path(sys.argv[1]).resolve()), Path(workspace))
