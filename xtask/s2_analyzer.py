"""Check a simulated battery's WSS conversation with FAN's pinned S2 Analyzer."""
import asyncio
import json
import os
import secrets
import ssl
import sys
import tempfile
import uuid
from datetime import UTC, datetime
from http.client import HTTPSConnection
from pathlib import Path

from websockets.asyncio.client import connect
from s2_analyzer_backend.device_connection.connection_adapter import WebSocketConnectionAdapter
from s2_analyzer_backend.device_connection.origin_type import S2OriginType
from s2_analyzer_backend.message_processor.message import Message
from s2_analyzer_backend.message_processor.message_processor import MessageParserProcessor

from doctor import ports, running, wait_for


async def inspect(uri, context, token):
    parser = MessageParserProcessor()
    session = uuid.uuid4()

    async def validate(origin, frame):
        result = await parser.process_message(Message(
            session_id=session, cem_id="analyzer", rm_id="battery", origin=origin, msg=frame
        ), asyncio.get_running_loop())
        assert result.s2_msg is not None and result.s2_validation_error is None, result.s2_validation_error

    async with asyncio.timeout(30), connect(uri, ssl=context, additional_headers={
        "Authorization": "Bearer " + token,
    }) as socket:
        transport = WebSocketConnectionAdapter(socket)

        async def send(kind, **fields):
            frame = dict(message_type=kind, **fields)
            if kind != "ReceptionStatus":
                frame["message_id"] = str(uuid.uuid4())
            await validate(S2OriginType.CEM, frame)
            await transport.send(json.dumps(frame))

        seen = set()
        actuator = None
        instruction = None
        while True:
            frame = json.loads(await transport.receive())
            await validate(S2OriginType.RM, frame)
            kind = frame["message_type"]
            seen.add(kind)
            if kind == "ReceptionStatus":
                assert frame["status"] == "OK", frame
                continue
            await send("ReceptionStatus", subject_message_id=frame["message_id"], status="OK")
            if kind == "Handshake":
                assert frame["role"] == "RM"
                # Select an actual advertised version understood by the independent stack.
                version = next(v for v in frame["supported_protocol_versions"] if v.lstrip("v") == "0.0.2-beta")
                await send("Handshake", role="CEM", supported_protocol_versions=[version])
                await send("HandshakeResponse", selected_protocol_version=version)
            elif kind == "ResourceManagerDetails":
                assert "FILL_RATE_BASED_CONTROL" in frame["available_control_types"]
                await send("SelectControlType", control_type="FILL_RATE_BASED_CONTROL")
            elif kind == "FRBC.SystemDescription":
                actuator, = frame["actuators"]
            elif kind == "FRBC.StorageStatus" and instruction is None:
                assert actuator and {"Handshake", "ResourceManagerDetails"} <= seen
                assert frame["present_fill_level"] > 0
                # The SunSpec battery reports SoC but provides no signed power setpoint.
                charge = next(m for m in actuator["operation_modes"] if m["diagnostic_label"] == "charge")
                instruction = str(uuid.uuid4())
                await send("FRBC.Instruction", id=instruction, actuator_id=actuator["id"],
                           operation_mode=charge["id"], operation_mode_factor=0.25,
                           execution_time=datetime.now(UTC).isoformat(), abnormal_condition=False)
            elif kind == "InstructionStatusUpdate" and frame["instruction_id"] == instruction:
                if frame["status_type"] in {"ABORTED", "REJECTED"}:
                    return



def run(binary, directory):
    api, modbus, ship = ports(3)
    config = directory / "house.toml"
    token = secrets.token_hex(24)
    env = dict(os.environ, EMS_ANALYZER_TRIAL=token)
    with (directory / "gateway.log").open("w") as log, running([
        binary, "house", "--listen", f"127.0.0.1:{modbus}", "--api", f"127.0.0.1:{api}",
        "--ship", f"127.0.0.1:{ship}", "--out", str(config),
    ], directory, env, log) as house:
        written = wait_for(config.read_text, house)
        config.write_text('store_path = "box.redb"\n[api]\ntls = true\ntoken = "env:EMS_ANALYZER_TRIAL"\n'
                          '[s2]\nenabled = true\nreport_every_secs = 1\n' + written)
        with running([binary, "run", "--config", str(config)], directory, env, log) as gateway:
            pem = wait_for(lambda: ssl.get_server_certificate(("127.0.0.1", api), timeout=2), gateway)
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
            context.load_verify_locations(cadata=pem)
            conn = HTTPSConnection("localhost", api, context=context, timeout=5)
            try:
                conn.request("PUT", "/v1/managers/analyzer", headers={
                    "Authorization": "Bearer " + token, "Content-Type": "application/json",
                })
                response = conn.getresponse()
                assert response.status in (200, 201), response.status
                manager = json.load(response)["token"]
            finally:
                conn.close()
            asyncio.run(inspect(f"wss://localhost:{api}/s2/battery", context, manager))
    print("S2 Analyzer validated the pinned WSS handshake, battery description, live fill level and undriven instruction outcome.")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as workspace:
        run(str(Path(sys.argv[1]).resolve()), Path(workspace))
