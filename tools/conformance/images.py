"""Build OCI archives and measure the four EMS ports on a private Docker network."""
import argparse
import contextlib
import hashlib
import json
import os
import re
import secrets
import subprocess
import sys
import tarfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
KIT = ROOT / "tools/conformance"
PYTHON = "python:3.13-alpine@sha256:9ef1a599f863231c3d91e941389bfdc4a2d6070557243d54df1a4e57015b960e"
ROLES = {"hemsd": "HEMS_TOKEN", "tariffd": "MCP_TOKEN"}


def docker(*arguments, capture=False, env=None):
    return subprocess.run(["docker", *arguments], check=True, cwd=ROOT, env=env,
                          stdout=subprocess.PIPE if capture else None, text=True).stdout


def inspect_archive(path, platform):
    def blob(archive, descriptor):
        digest = descriptor["digest"]
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
            raise ValueError("Invalid OCI digest")
        data = archive.extractfile("blobs/sha256/" + digest[7:]).read()
        if len(data) != descriptor["size"] or hashlib.sha256(data).hexdigest() != digest[7:]:
            raise ValueError("OCI content differs from the manifest")
        return json.loads(data)

    with tarfile.open(path) as archive:
        index = json.load(archive.extractfile("index.json"))
        [descriptor] = index["manifests"]
        manifest = blob(archive, descriptor)
        config = blob(archive, manifest["config"])
        if (f"{config['os']}/{config['architecture']}" != platform or
                config["config"]["User"].split(":")[0] not in ("65532", "nonroot")):
            raise ValueError("Image architecture or runtime user does not match the profile")
        return {"manifest": descriptor["digest"], "config": manifest["config"]["digest"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("linux/arm64", "linux/amd64"))
    parser.add_argument("--output", type=Path, default=ROOT / "target/interfaces")
    parser.add_argument("--archives", action="store_true", help="qualify existing OCI archives in --output")
    args = parser.parse_args()
    platform = args.platform or "linux/" + docker("version", "--format", "{{.Server.Arch}}", capture=True).strip()
    dest = args.output.resolve()
    dest.mkdir(parents=True, exist_ok=True)
    evidence = dest / "evidence.json"
    evidence.unlink(missing_ok=True)
    subprocess.run([sys.executable, KIT / "contracts.py"], check=True)
    network = "ems-ports-" + secrets.token_hex(6)
    env = dict(os.environ, **{variable: secrets.token_urlsafe(32) for variable in ROLES.values()})
    if not args.archives:
        # Qualify both targets from one BuildKit solve and one source snapshot.
        targets = {role: {"target": role, "platforms": [platform],
                          "output": [{"type": "oci", "dest": str(dest / f"{role}.tar")}]}
                   for role in ROLES}
        subprocess.run(["docker", "buildx", "bake", "-f", "-", "--provenance=false", *ROLES],
                       cwd=ROOT, input=json.dumps({"target": targets}), text=True, check=True)
    identities = {}
    runtime_ids = {}
    for role in ROLES:
        archive = dest / f"{role}.tar"
        image = inspect_archive(archive, platform)
        loaded = docker("load", "-i", str(archive), capture=True)
        # The containerd image store uses the manifest ID. The classic store uses
        # the configuration ID. Both must refer to the measured archive.
        marker = "Loaded image ID: "
        runtime_id = next((line[len(marker):] for line in loaded.splitlines() if line.startswith(marker)), "")
        if runtime_id not in (image["manifest"], image["config"]):
            raise ValueError("Docker imported an unexpected image identity")
        identities[role] = image
        runtime_ids[role] = runtime_id

    with contextlib.ExitStack() as cleanup:
        docker("network", "create", "--internal", network, capture=True)
        cleanup.callback(docker, "network", "rm", network, capture=True)
        for role, variable in ROLES.items():
            name = f"{network}-{role}"
            docker("run", "-d", "--pull", "never", "--platform", platform, "--network", network, "--network-alias", role,
                   "--name", name, "-e", variable, "-v", f"{KIT}/{role}.toml:/etc/ems/{role}.toml:ro",
                   runtime_ids[role], capture=True, env=env)
            cleanup.callback(docker, "rm", "-fv", name, capture=True)
            running = docker("inspect", "--format", "{{.Image}}", name, capture=True).strip()
            if running not in identities[role].values():
                raise ValueError("Container identity differs from the OCI archive")
        probe_name = network + "-probe"
        # Only the bridge can reach the probe and both services.
        command = ["run", "--rm", "--name", probe_name, "--network", network, "--network-alias", "probe",
                   "-e", "HEMS_TOKEN", "-e", "MCP_TOKEN", "-v", f"{KIT}:/kit/tools/conformance:ro",
                   "-v", f"{ROOT}/platform.toml:/kit/platform.toml:ro", PYTHON, "python3",
                   "/kit/tools/conformance/probe.py", "--gateway", "http://hemsd:8080",
                   "--tariff", "http://tariffd:8081", "--callback", "http://probe:8085", "--wait", "45"]
        checks = json.loads(docker(*command, capture=True, env=env))
    profile = tomllib.loads((ROOT / "platform.toml").read_text())
    owners = {"rest": ["hemsd"], "cloudevents": ["hemsd"], "webhooks": ["hemsd"], "mcp": ["tariffd"]}
    if profile["ports"] != owners or checks != dict.fromkeys(owners, "passed"):
        raise ValueError("Measured ports differ from the declared provider matrix")
    report = {"profile": "EMS household gateway and tariffs", "platform": platform,
              "images": identities, "ports": {port: {"provider": owners[port][0], "result": result}
                                             for port, result in checks.items()}}
    pending = dest / f"{network}.json"
    pending.write_text(json.dumps(report, indent=2) + "\n")
    pending.replace(evidence)
    print(evidence.read_text())


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(f"Image qualification stopped ({error.cmd[0]} exited {error.returncode}).")
    except (ValueError, OSError, tarfile.TarError) as error:
        sys.exit(f"Image qualification stopped: {error}")
