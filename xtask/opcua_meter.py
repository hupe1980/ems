# /// script
# requires-python = ">=3.11"
# dependencies = ["asyncua==2.1.0", "cryptography==50.0.2"]
# ///
"""Commission a source-timed industrial meter against an encrypted, independent OPC UA peer."""
import asyncio
import json
import logging
import sys
from datetime import UTC, datetime, timedelta
from ipaddress import ip_address
from pathlib import Path
from tempfile import TemporaryDirectory

from asyncua import Server, ua
from asyncua.common.utils import ServiceError
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

EXAMPLE = Path("services/cemsd/examples/opcua")
CLI = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/cemsd").resolve()


def identity(folder, name, uri, usage):
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, name)])
    now = datetime.now(UTC)
    cert = (x509.CertificateBuilder().subject_name(subject).issuer_name(subject)
            .public_key(key.public_key()).serial_number(x509.random_serial_number())
            .not_valid_before(now - timedelta(days=1)).not_valid_after(now + timedelta(days=1))
            .add_extension(x509.SubjectAlternativeName([x509.UniformResourceIdentifier(uri),
                                                        x509.IPAddress(ip_address("127.0.0.1"))]), False)
            .add_extension(x509.KeyUsage(True, True, True, True, False, False, False, None, None), True)
            .add_extension(x509.BasicConstraints(ca=False, path_length=None), True)
            .add_extension(x509.ExtendedKeyUsage([usage]), False).sign(key, hashes.SHA256()))
    (folder / f"{name}.der").write_bytes(cert.public_bytes(serialization.Encoding.DER))
    (folder / f"{name}-key.der").write_bytes(key.private_bytes(serialization.Encoding.DER,
                                               serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))


async def invoke(*args, success=True):
    child = await asyncio.create_subprocess_exec(str(CLI), *map(str, args),
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
    try:
        output, error = await asyncio.wait_for(child.communicate(), 30)
    finally:
        if child.returncode is None:
            child.kill()
            await child.wait()
    assert (child.returncode == 0) == success, (args[0], child.returncode, error)
    return output


async def rehearse(folder):
    definition = json.loads((EXAMPLE / "contract.json").read_text())
    window = json.loads((EXAMPLE / "window.json").read_text())
    readings = [json.loads(line) for line in (EXAMPLE / "readings.ndjson").read_text().splitlines()]
    client_uri = "urn:ems:commissioner"
    identity(folder, "server", definition["applicationUri"], ExtendedKeyUsageOID.SERVER_AUTH)
    identity(folder, "client", client_uri, ExtendedKeyUsageOID.CLIENT_AUTH)
    identity(folder, "other", definition["applicationUri"], ExtendedKeyUsageOID.SERVER_AUTH)
    server = Server()
    await server.init()
    await server.set_application_uri(definition["applicationUri"])
    server.set_endpoint("opc.tcp://127.0.0.1:0/")
    await server.load_certificate(folder / "server.der")
    await server.load_private_key(folder / "server-key.der")
    server.set_security_policy([ua.SecurityPolicyType.Basic256Sha256_SignAndEncrypt])
    server.set_identity_tokens([ua.AnonymousIdentityToken])
    trusted = (folder / "client.der").read_bytes()

    async def validate(cert, application):
        if cert.public_bytes(serialization.Encoding.DER) != trusted or application.ApplicationUri != client_uri:
            raise ServiceError(ua.StatusCodes.BadCertificateUntrusted)

    server.set_certificate_validator(validate)
    ns = await server.register_namespace("urn:ems:plant")
    node = await server.nodes.objects.add_variable(ua.NodeId("GridImportWh", ns), "Import",
                                                    ua.Variant(0, ua.VariantType.UInt64))
    await node.write_value_rank(-1)
    unit = ua.EUInformation(NamespaceUri="http://www.opcfoundation.org/UA/units/un/cefact",
                            UnitId=int.from_bytes(b"WHR", "big"))
    prop = await node.add_property(ua.NodeId("Unit", ns), ua.QualifiedName("EngineeringUnits", 0),
                                  ua.Variant(unit, ua.VariantType.ExtensionObject), datatype=ua.ObjectIds.EUInformation)
    await prop.write_value_rank(-1)
    changing_units = False
    it = iter(readings)

    def next_boundary(_node_id, _attr):
        r = next(it)
        if changing_units:
            unit.UnitId = int.from_bytes(b"KWH", "big")
        return ua.DataValue(ua.Variant(r["value"], ua.VariantType.UInt64),
                            SourceTimestamp=datetime.fromisoformat(r["sourceAt"]))

    server.set_attribute_value_callback(node.nodeid, next_boundary)
    server.set_attribute_value_callback(prop.nodeid, lambda _n, _a:
        ua.DataValue(ua.Variant(unit, ua.VariantType.ExtensionObject)))
    async with server:
        capture = ["capture-meter", "--contract", EXAMPLE / "contract.json",
                   "--endpoint", f"opc.tcp://127.0.0.1:{server.bserver.port}/", "--client-uri", client_uri,
                   "--client-cert", folder / "client.der", "--client-key", folder / "client-key.der"]
        assert not await invoke(*capture, "--server-cert", folder / "other.der", success=False)
        captured = await invoke(*capture, "--server-cert", folder / "server.der", "--samples", 3, "--period-seconds", 1)
        # A later read now sees different units. A failed batch must emit no partial NDJSON.
        it = iter(readings)
        changing_units = True
        assert not await invoke(*capture, "--server-cert", folder / "server.der", "--samples", 2,
                                "--period-seconds", 1, success=False)
        assert unit.UnitId == int.from_bytes(b"KWH", "big")

    rows = [json.loads(line) for line in captured.splitlines()]
    assert [(r["value"], r["sourceAt"]) for r in rows] == [(r["value"], r["sourceAt"]) for r in readings]
    path = folder / "readings.ndjson"
    path.write_bytes(captured)
    contract_path = folder / "contract.json"
    window_path = folder / "window.json"
    contract_path.write_text(json.dumps(definition))
    window_path.write_text(json.dumps(window))
    review = ["commission-meter", "--contract", contract_path, "--window", window_path, "--readings", path]
    result = json.loads(await invoke(*review))
    assert result["meter"]["first_exclusion"] is None
    assert result["meter"]["usable_points"] == 3
    assert [q["difference_wh"] for q in result["reconciliation"]["intervals"]] == [0, 1]
    html = (await invoke(*review, "--format", "html")).decode()
    assert "02:45 +02:00" in html and "02:00 +01:00" in html
    rows[1].update(status=0x40900000)
    del rows[1]["value"]
    path.write_text(''.join(json.dumps(r) + '\n' for r in rows))
    result = json.loads(await invoke(*review, success=False))
    assert result["meter"]["first_exclusion"] == {
        "line": 2, "reason": "status", "status_code": 0x40900000,
        "source_at": rows[1]["sourceAt"], "detail": "OPC UA status 0x40900000",
    }
    assert result["meter"]["excluded_status"] == 1
    assert [q["status"] for q in result["reconciliation"]["intervals"]] == ["missing_boundary"] * 2
    assert b"Go to first exception" in await invoke(*review, "--format", "html", success=False)
    # A 32-bit Float at 2^24 cannot distinguish the next Wh.
    float_type = {"namespace": "http://opcfoundation.org/UA/", "id": "i=10"}
    float_contract = json.loads((EXAMPLE / "contract.json").read_text())
    float_contract["signals"][0]["dataType"] = float_type
    contract_path.write_text(json.dumps(float_contract))
    floating = {**readings[0], "dataType": float_type, "valueBits": 0x4B800000}
    del floating["value"]
    path.write_text(json.dumps(floating) + "\n")
    result = json.loads(await invoke(*review, success=False))
    assert result["meter"]["excluded_wh"] == 1 and result["meter"]["usable_points"] == 0
    assert result["meter"]["first_exclusion"]["reason"] == "insufficient_resolution"
    # The original Double regression uses an exactly representable negative whole Wh.
    float_contract["signals"][0]["dataType"]["id"] = "i=11"
    contract_path.write_text(json.dumps(float_contract))
    floating["valueBits"] = "bff0000000000000"
    path.write_text("\n" + json.dumps(floating) + "\n")
    rejection = json.loads(await invoke(*review, success=False))["meter"]["first_exclusion"]
    assert rejection == {"line": 2, "source_at": floating["sourceAt"], "reason": "negative_wh",
                         "detail": "negative cumulative Wh"}
    html = await invoke(*review, "--format", "html", success=False)
    assert b"Capture line 2" in html and b"negative cumulative Wh" in html
    for bits, reason in [("3fe0000000000000", "fractional_wh"), ("7ff8000000000000", "non_finite_wh")]:
        floating["valueBits"] = bits
        path.write_text(json.dumps(floating))
        result = json.loads(await invoke(*review, success=False))
        assert result["meter"]["first_exclusion"]["reason"] == reason
    floating["sourceAt"] = None
    path.write_text(json.dumps(floating))
    rejection = json.loads(await invoke(*review, success=False))["meter"]["first_exclusion"]
    assert rejection["reason"] == "missing_source_time" and rejection["source_at"] is None
    # Treat asset labels as plain text in the standalone HTML.
    label = "Incomer <north>"
    for r in rows:
        r["device"] = label
    definition["device"] = window["meter"]["device"] = label
    path.write_text(''.join(json.dumps(r) + '\n' for r in rows))
    contract_path.write_text(json.dumps(definition))
    window_path.write_text(json.dumps(window))
    html = await invoke(*review, "--format", "html", success=False)
    assert b"Incomer &lt;north&gt;" in html and label.encode() not in html
    window["meter"]["device"] = "Different incomer"
    window_path.write_text(json.dumps(window))
    assert not await invoke(*review, success=False)
    path.write_bytes(b' ' * (4 * 1024 * 1024 + 1))
    assert not await invoke(*review, success=False)
    print("Pinned meter captured three exact boundaries. Two independently closed quarters agreed within 1 Wh. Wrong pin, units, meter identity and incomplete evidence were refused.")


if __name__ == "__main__":
    logging.getLogger("asyncua").setLevel(logging.CRITICAL)
    with TemporaryDirectory() as work:
        asyncio.run(rehearse(Path(work)))
