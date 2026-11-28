"""Exercise two disposable gateway builds against a loopback fleet."""

import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.parse

# Changing the build changes the bytes hashed by the process that actually starts.
GATEWAY = '''import hashlib, json, os, pathlib, sys, time, urllib.request, uuid
BUILD = {build!r}
class Direct(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None
boot = str(uuid.uuid4())
digest = hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()
if sys.argv[1] == "verify":
    payload = dict(phase="verified", initial_boot=boot, candidate=sys.argv[2], verdict=sys.argv[3])
else:
    feeder = pathlib.Path(os.environ["CANARY_FEEDER"])
    if feeder.exists():
        sample = json.loads(feeder.read_text())
        ready = 0 <= int(time.time()) - sample["at"] <= 60 and isinstance(sample["watts"], int)
    else:
        ready = False
    payload = dict(phase="running", sent_at=int(time.time()), boot=boot, digest=digest, ready=ready, rollback=sys.argv[1]=="rollback")
payload["run"] = os.environ["CANARY_RUN"]
request = urllib.request.Request(os.environ["CANARY_RECEIVER"], data=json.dumps(payload).encode(), headers={{"Authorization":"Bearer " + os.environ["FLEET_SITE_TOKEN"], "Content-Type":"application/json"}})
with urllib.request.build_opener(urllib.request.ProxyHandler({{}}), Direct).open(request, timeout=5) as response:
    assert response.status == 204
'''


def run():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--site", required=True)
    parser.add_argument("--case", choices=["ready", "rollback", "wrong-build", "unready", "wrong-target", "rejected"], default="ready")
    parser.add_argument("--bin-dir", type=Path, default=Path("target/debug"))
    args = parser.parse_args()
    url = urllib.parse.urlsplit(args.url)
    if url.scheme != "http" or not ipaddress.ip_address(url.hostname or "").is_loopback or url.path not in ("", "/") or url.username or url.password or url.fragment or url.query:
        parser.error("Use a loopback HTTP origin.")
    if not os.environ.get("FLEET_SITE_TOKEN"):
        parser.error("Set FLEET_SITE_TOKEN to the enrolled simulator credential.")
    verifier = args.bin_dir / "examples" / "canary_manifest"
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        old, new, release = root / "current.py", root / "candidate.py", root / "release.json"
        old.write_text(GATEWAY.format(build="current"))
        new.write_text(GATEWAY.format(build="candidate"))
        component = "tariffd" if args.case == "wrong-target" else "hemsd"
        release.write_bytes(subprocess.check_output([verifier, "sign", new, component]))
        digest = hashlib.sha256(new.read_bytes()).hexdigest()
        if args.case == "rejected":
            new.write_text(new.read_text() + "# damaged in transit\n")
        verdict = subprocess.check_output([verifier, "verify", new, release], text=True).strip()
        result = subprocess.check_output([args.bin_dir / "fleetd", "--config", args.config, "rehearse", "--site", args.site, "--digest", digest], text=True)
        run_id = result.splitlines()[-1]
        feeder = root / "feeder.json"
        if args.case != "unready":
            feeder.write_text(json.dumps({"at": int(time.time()), "watts": 812}))
        env = dict(os.environ, CANARY_FEEDER=str(feeder), CANARY_RUN=run_id, CANARY_RECEIVER=f"{args.url.rstrip('/')}/v1/rehearsals/{run_id}/observations")
        subprocess.run([sys.executable, old, "verify", digest, verdict], env=env, check=True)
        if verdict == "verified":
            build = old if args.case in ("wrong-build", "rollback") else new
            subprocess.run([sys.executable, build, "rollback" if args.case == "rollback" else "start"], env=env, check=True)
        print(json.dumps({"run": run_id, "candidate": digest, "verification": verdict, "case": args.case}))


if __name__ == "__main__":
    run()
