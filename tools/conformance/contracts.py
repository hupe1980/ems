"""Export the gateway's structured CloudEvents as portable JSON Schema 2020-12."""
import argparse
import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVENTS = ROOT / "schemas/events"


def standalone(document, kind):
    operation = document["webhooks"][kind]["post"]
    envelope = operation["requestBody"]["content"]["application/cloudevents+json"]["schema"]
    definitions = {}

    def local(value):
        if isinstance(value, list):
            return [local(item) for item in value]
        if not isinstance(value, dict):
            return value
        result = {key: local(child) for key, child in value.items()}
        if "$ref" in result:
            prefix = "#/components/schemas/"
            reference = result["$ref"]
            if not reference.startswith(prefix):
                raise ValueError(f"Unexpected reference {reference}")
            name = reference[len(prefix):]
            result["$ref"] = "#/$defs/" + name
            if name not in definitions:
                definitions[name] = None
                definitions[name] = local(document["components"]["schemas"][name])
        return result

    schema_uri = envelope["properties"]["dataschema"]["const"]
    if not schema_uri.endswith("#/properties/data"):
        raise ValueError(f"{kind} must identify its payload schema")
    return {"$id": schema_uri.split("#")[0], "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": kind, "description": operation["summary"],
            **local(envelope), "$defs": definitions}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="regenerate the declared event schemas")
    args = parser.parse_args()
    profile = tomllib.loads((ROOT / "platform.toml").read_text())
    contract = profile["contracts"]
    document = json.loads((ROOT / contract["openapi"]).read_text())
    expected = {f"schemas/events/{kind}.json" for kind in document["webhooks"]}
    if sorted(contract["event_schemas"]) != sorted(expected):
        sys.exit("Update platform.toml to declare each gateway event exactly once.")
    for kind in sorted(document["webhooks"]):
        path = EVENTS / f"{kind}.json"
        content = json.dumps(standalone(document, kind), indent=2, sort_keys=True, ensure_ascii=False) + "\n"
        if args.write:
            path.write_text(content)
        elif not path.exists() or path.read_text() != content:
            sys.exit(f"Regenerate {path.name} with python3 tools/conformance/contracts.py --write")
    if {str(p.relative_to(ROOT)) for p in EVENTS.glob("*.json")} != expected:
        sys.exit("Unexpected event schema in schemas/events")
    print(f"{len(expected)} event schemas match the gateway contract.")


if __name__ == "__main__":
    main()
