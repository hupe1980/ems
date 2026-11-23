"""Read retained power history while a gateway manages the loopback virtual house."""

import json
import os
import secrets
import sys
import tempfile
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen

from doctor import ports, running, wait_for


def trial(binary, directory):
    api, modbus, ship = ports(3)
    token = secrets.token_hex(24)
    env = dict(os.environ, EMS_HISTORY_TRIAL=token)
    config = directory / "house.toml"

    def get(path):
        request = Request(f"http://127.0.0.1:{api}/v1/{path}",
                          headers={"Authorization": f"Bearer {token}"})
        with urlopen(request, timeout=10) as response:
            return json.load(response)

    with (directory / "trial.log").open("w") as log, running([
        binary, "house", "--listen", f"127.0.0.1:{modbus}", "--ship", f"127.0.0.1:{ship}",
        "--api", f"127.0.0.1:{api}", "--out", str(config)
    ], directory, env, log) as house:
        written = wait_for(config.read_text, house)
        config.write_text('[api]\ntoken = "env:EMS_HISTORY_TRIAL"\n[series]\npath = "series"\n' + written)
        with running([binary, "run", "--config", str(config)], directory, env, log) as gateway:
            def ready():
                try:
                    return get("series/grid")["readings"]
                except OSError:
                    return None

            wait_for(ready, gateway)
            initial = get("status")
            stamps = {initial["at"]}
            reads = 0
            peak_ms = 0
            until = time.monotonic() + 8
            while time.monotonic() < until:
                now = datetime.now(UTC)
                # Ask for the entire retained horizon, including the current open bucket.
                query = urlencode({"from": (now - timedelta(days=1095)).isoformat(),
                                   "to": now.isoformat()})
                started = time.monotonic()
                history = get("series/grid?" + query)
                peak_ms = max(peak_ms, (time.monotonic() - started) * 1000)
                assert history["resolution"] == "quarter_hours"
                assert any(row.get("samples", 0) > 0 for row in history["readings"])
                stamps.add(get("status")["at"])
                reads += 1
                time.sleep(0.05)
            final = get("status")
            assert len(stamps) >= 3, "Control clock did not advance during history reads"
            assert final["overruns"] == initial["overruns"], "Control exceeded its tick budget"
            print(f"{reads} reads, {len(stamps)} distinct control ticks, zero new overruns, "
                  f"slowest request {peak_ms:.1f} ms (fresh virtual house).")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as workspace:
        trial(str(Path(sys.argv[1]).resolve()), Path(workspace))
