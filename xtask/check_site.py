"""Check the links and discovery metadata in a rendered Zola site."""

import json
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urljoin, urlsplit
from xml.etree import ElementTree


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.ids = set()
        self.links = []
        self.canonicals = []
        self.structured = False
        self.description = ""
        self.noindex = False
        self.in_schema = False
        self.feed(path.read_text())
        self.close()

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        for key in ("href", "src", "poster", "data-library", "data-index"):
            if attrs.get(key):
                self.links.append(attrs[key])
        if tag == "link" and attrs.get("rel") == "canonical":
            self.canonicals.append(attrs["href"])
        if tag == "meta" and attrs.get("name") == "description":
            self.description = attrs.get("content", "").strip()
        if tag == "meta" and attrs.get("name") == "robots":
            self.noindex = "noindex" in attrs.get("content", "").lower()
        if tag == "script":
            self.in_schema = attrs.get("type") == "application/ld+json"

    def handle_endtag(self, tag):
        if tag == "script":
            self.in_schema = False

    def handle_data(self, data):
        if self.in_schema:
            json.loads(data)
            self.structured = True


def check(root, base):
    base = base.rstrip("/") + "/"
    origin = urlsplit(base)
    pages = {p: Page(p) for p in [*root.rglob("index.html"), root / "404.html"]}
    errors = set()
    for schema in (Path(__file__).resolve().parents[1] / "schemas/events").glob("*.json"):
        published = root / "contracts/events/v1" / schema.name
        if not published.is_file() or published.read_bytes() != schema.read_bytes():
            errors.add(f"Missing or stale published event schema: {schema.name}")
    indexable = set()
    for path, page in pages.items():
        relative = path.relative_to(root).as_posix().removesuffix("index.html")
        url = urljoin(base, relative)
        if path.name != "404.html":
            indexable.add(url)
            if page.canonicals != [url] or page.noindex or not page.structured or not page.description:
                errors.add(f"Invalid search metadata on {url}")
        elif not page.noindex or page.canonicals:
            errors.add("The 404 page needs noindex and must omit a canonical URL")

        for link in page.links:
            target = urlsplit(urljoin(url, link))
            if target.netloc != origin.netloc or target.scheme not in {"http", "https"}:
                continue
            if not target.path.startswith(origin.path):
                errors.add(f"{relative} links outside the project at {link}")
                continue
            file = root / unquote(target.path[len(origin.path):])
            if file.is_dir():
                file = file / "index.html"
            if not file.is_file():
                errors.add(f"{relative} links to missing file {link}")
            elif file in pages and target.fragment and unquote(target.fragment) not in pages[file].ids:
                errors.add(f"{relative} links to missing fragment {link}")

    sitemap = ElementTree.parse(root / "sitemap.xml")
    indexed = {node.text for node in sitemap.findall(".//{*}loc")}
    if indexed != indexable:
        errors.add(f"Sitemap and canonical pages differ: {sorted(indexed ^ indexable)}")
    if f"Sitemap: {base}sitemap.xml" not in (root / "robots.txt").read_text().splitlines():
        errors.add("robots.txt needs the current sitemap URL")
    if errors:
        raise SystemExit("\n".join(sorted(errors)))
    print(f"Verified {len(pages)} pages, local links, anchors, canonical URLs and sitemap.")


if __name__ == "__main__":
    import tomllib

    site = Path(__file__).resolve().parents[1] / "site"
    base_url = tomllib.loads((site / "config.toml").read_text())["base_url"]
    check(Path(sys.argv[1]) if len(sys.argv) > 1 else site / "public", base_url)
