"""Run the API guide's automation in an isolated Home Assistant container."""

import asyncio
import json
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile

import aiohttp
import yaml


async def exercise(config):
    base = "http://127.0.0.1:8123"
    webhook = config["automation"][0]["triggers"][0]["webhook_id"]
    async with aiohttp.ClientSession(base, raise_for_status=True) as client:
        for _ in range(90):
            try:
                async with client.get("/api/onboarding"):
                    break
            except aiohttp.ClientError:
                await asyncio.sleep(1)
        else:
            raise RuntimeError("Home Assistant did not start")
        async with client.post("/api/onboarding/users", json={
            "client_id": base + "/", "name": "EMS", "username": "ems",
            "password": secrets.token_urlsafe(32), "language": "en",
        }) as response:
            code = (await response.json())["auth_code"]
        async with client.post("/auth/token", data={
            "grant_type": "authorization_code", "code": code, "client_id": base + "/",
        }) as response:
            token = (await response.json())["access_token"]
        # Onboarding accepts requests before Home Assistant attaches its automations.
        headers = {"Authorization": "Bearer " + token}
        for _ in range(60):
            async with client.get("/api/config", headers=headers) as response:
                if (await response.json())["state"] == "RUNNING":
                    break
            await asyncio.sleep(0.5)
        else:
            raise RuntimeError("Home Assistant did not finish startup")

        async with client.ws_connect("/api/websocket") as ws:
            assert (await ws.receive_json())["type"] == "auth_required"
            await ws.send_json({"type": "auth", "access_token": token})
            assert (await ws.receive_json())["type"] == "auth_ok"
            for index, (kind, data) in enumerate([
                ("de.ems.test.ping", {"test": True}),
                ("de.ems.grid.lpc.limit.received", {"limit_w": 4200, "minimum_w": 1400}),
                ("de.ems.grid.lpc.limit.received", {"limit_w": 3200, "minimum_w": None}),
            ], start=1):
                body = {"specversion": "1.0", "id": str(index), "source": "ems://sites/demo",
                        "type": kind, "data": data}
                async with client.post("/api/webhook/" + webhook, data=json.dumps(body),
                                       headers={"Content-Type": "application/cloudevents+json"}):
                    pass
                for poll in range(20):
                    await ws.send_json({"id": index * 100 + poll, "type": "persistent_notification/get"})
                    result = await ws.receive_json()
                    assert result["success"], result
                    messages = [n["message"] for n in result["result"] if n["title"] == "§ 14a"]
                    if index == 1:
                        assert not messages
                        break
                    expected = "4.2 kW" if index == 2 else "3.2 kW"
                    if any(expected in m for m in messages):
                        if index == 2:
                            assert any("1.4 kW" in m for m in messages)
                        else:
                            assert "Reduced to 3.2 kW." in [m.strip() for m in messages]
                        break
                    await asyncio.sleep(0.25)
                else:
                    raise RuntimeError(f"Event {index} produced unexpected notifications: {messages!r}")


def main():
    page = Path(sys.argv[1]).read_text()
    example = page.split("### Home Assistant\n", 1)[1].split("```yaml\n", 1)[1].split("\n```", 1)[0]
    config = yaml.safe_load(example)
    with tempfile.TemporaryDirectory() as directory:
        Path(directory, "configuration.yaml").write_text(
            "homeassistant:\n  name: EMS\n  time_zone: Europe/Berlin\n"
            "http:\n  server_host: 127.0.0.1\napi:\nauth:\nonboarding:\nperson:\n"
            "frontend:\nwebhook:\npersistent_notification:\n" + example
        )
        process = subprocess.Popen([sys.executable, "-m", "homeassistant", "--config", directory])
        try:
            asyncio.run(asyncio.wait_for(exercise(config), timeout=120))
        finally:
            process.terminate()
            process.wait(timeout=30)
    print("Home Assistant accepted the documented reduction and ignored the ping.")


if __name__ == "__main__":
    main()
