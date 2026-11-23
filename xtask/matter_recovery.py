# /// script
# requires-python = ">=3.11"
# dependencies = ["websockets==15.0.1"]
# ///
"""Exercise the household gateway against a loopback Matter Server WebSocket peer."""
import json
import os
import secrets
import subprocess
import sys
import tempfile
from contextlib import contextmanager
from datetime import datetime
from pathlib import Path
from threading import Thread
from urllib.error import URLError
from urllib.request import Request, urlopen

from websockets.sync.server import serve

from doctor import ports, running, wait_for


@contextmanager
def peer(port, milliwatts):
    def reply(socket):
        assert socket.request.path == "/ws"
        socket.send(json.dumps({"compressed_fabric_id": 48879, "schema_version": 15,
                                "min_supported_schema_version": 11}))
        command = json.loads(socket.recv(timeout=5))
        assert command["command"] == "read_attribute"
        assert command["args"] == {"node_id": 10, "attribute_path": "2/144/8"}
        socket.send(json.dumps({"message_id": command["message_id"],
                                "result": {"2/144/8": milliwatts}}))

    with serve(reply, "127.0.0.1", port) as server:
        thread = Thread(target=server.serve_forever)
        thread.start()
        try:
            yield
        finally:
            server.shutdown()
            thread.join()


def trial(binary, directory):
    api, modbus, ship, matter = ports(4)
    token = secrets.token_hex(24)
    env = dict(os.environ, EMS_MATTER_TRIAL=token)
    config = directory / "house.toml"
    run = [binary, "run", "--config", str(config)]
    request = Request(f"http://127.0.0.1:{api}/v1/home",
                      headers={"Authorization": f"Bearer {token}"})

    def probe():
        return subprocess.run([binary, "matter-read", "--config", str(config)], env=env,
                              cwd=directory, capture_output=True, text=True, timeout=12)

    def observation(gateway, state):
        def read():
            try:
                with urlopen(request, timeout=2) as response:
                    sample = json.load(response)["matter"]
            except URLError as error:
                if isinstance(error.reason, ConnectionRefusedError):
                    return None
                raise
            return sample if sample["state"] == state else None

        sample = wait_for(read, gateway)
        assert (sample["name"], sample["node"], sample["endpoint"]) == ("Washer", "a", 2)
        return sample

    with (directory / "trial.log").open("w") as log, running([
        binary, "house", "--listen", f"127.0.0.1:{modbus}", "--ship", f"127.0.0.1:{ship}",
        "--api", f"127.0.0.1:{api}", "--out", str(config)
    ], directory, env, log) as house:
        written = wait_for(config.read_text, house)
        config.write_text('[api]\ntoken = "env:EMS_MATTER_TRIAL"\n' + written
                          + f'\n[matter]\nname = "Washer"\nfabric = "beef"\nnode = "a"\nendpoint = 2\nport = {matter}\n')
        with running(run, directory, env, log) as gateway:
            with peer(matter, -12500):
                result = probe()
                result.check_returncode()
                cli = json.loads(result.stdout)
                initial = observation(gateway, "current")
                for field in ("name", "node", "endpoint", "state", "watts"):
                    assert initial[field] == cli[field], field
                assert initial["watts"] == -12.5
            stale = observation(gateway, "stale")
            assert stale == {**initial, "state": "stale"}
            failed = probe()
            assert failed.returncode != 0 and not failed.stdout and "Matter Server" in failed.stderr
            with peer(matter, None):
                unknown = observation(gateway, "unknown")
                assert unknown["watts"] is None
                result = probe()
                result.check_returncode()
                observed = json.loads(result.stdout)
                assert observed["state"] == "unknown" and observed["watts"] is None
            with peer(matter, 0):
                zero = observation(gateway, "current")
                assert zero["watts"] == 0
        with running(run, directory, env, log) as restarted:
            empty = observation(restarted, "unavailable")
            assert empty["watts"] is None and empty["received_at"] is None
            with peer(matter, 2750):
                recovered = observation(restarted, "current")
                assert recovered["watts"] == 2.75
        times = [datetime.fromisoformat(row["received_at"]) for row in (initial, unknown, zero, recovered)]
        assert all(t.tzinfo for t in times)
        assert all(a < b for a, b in zip(times, times[1:]))
        print("Matter recovery passed: -12.5 W, stale, explicit null, zero, offline restart, +2.75 W.")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as workspace:
        trial(str(Path(sys.argv[1]).resolve()), Path(workspace))
