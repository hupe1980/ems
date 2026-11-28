"""Regression tests for false passes in the independent release verifier."""
import base64
import datetime
import hashlib
import hmac
import io
import json
import tarfile
import tempfile
import time
import unittest
from email.message import Message
from pathlib import Path

from images import inspect_archive
from probe import Failure, PING, sse_result, verify_delivery


class ReleaseChecks(unittest.TestCase):
    def test_callback_binds_wire_bytes_and_household(self):
        key = b"a fixture key for the receiver check"
        secret = "whsec_" + base64.b64encode(key).decode()
        stamp = str(int(time.time()))
        event = {"specversion": "1.0", "id": "fixture", "type": PING, "source": "ems://sites/house",
                 "dataschema": f"https://hupe1980.github.io/ems/contracts/events/v1/{PING}.json#/properties/data",
                 "time": datetime.datetime.now(datetime.timezone.utc).isoformat(), "data": {"test": True}}
        body = json.dumps(event).encode()
        headers = Message()
        headers["Content-Type"] = "application/cloudevents+json"
        headers["webhook-id"] = "fixture"
        headers["webhook-timestamp"] = stamp
        mac = hmac.digest(key, b"fixture." + stamp.encode() + b"." + body, "sha256")
        headers["webhook-signature"] = "v1," + base64.b64encode(mac).decode()
        verify_delivery((headers, body), secret, "house")
        for size in (23, 65):
            outside_profile = "whsec_" + base64.b64encode(bytes(size)).decode("ascii")
            with self.assertRaisesRegex(Failure, "key length"):
                verify_delivery((headers, body), outside_profile, "house")
        with self.assertRaisesRegex(Failure, "signature"):
            verify_delivery((headers, body + b" "), secret, "house")
        with self.assertRaisesRegex(Failure, "site"):
            verify_delivery((headers, body), secret, "neighbour")

    def test_sse_ignores_empty_events_and_matches_the_request(self):
        stream = io.BytesIO(b': keep-alive\n\ndata: \n\ndata: {"id":2}\n\n'
                            b'data: {"id":3,\r\ndata: "result":{}}\r\n\r\nstill open')
        self.assertEqual(json.loads(sse_result(stream, 3)), {"id": 3, "result": {}})
        self.assertEqual(stream.read(), b"still open")

    def test_archive_substitution_cannot_reuse_an_approved_digest(self):
        approved = b'{"schemaVersion":2}'
        changed = b'{"schemaVersion":1}'
        digest = hashlib.sha256(approved).hexdigest()
        index = {"manifests": [{"digest": "sha256:" + digest, "size": len(approved)}]}
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "candidate.tar"
            with tarfile.open(path, "w") as archive:
                for name, content in (("index.json", json.dumps(index).encode()),
                                      ("blobs/sha256/" + digest, changed)):
                    entry = tarfile.TarInfo(name)
                    entry.size = len(content)
                    archive.addfile(entry, io.BytesIO(content))
            with self.assertRaisesRegex(ValueError, "OCI content differs"):
                inspect_archive(path, "linux/amd64")
