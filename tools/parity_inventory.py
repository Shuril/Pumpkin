#!/usr/bin/env python3
"""Build a lightweight, deterministic vanilla/Pumpkin parity inventory.

This tool deliberately has no third-party dependencies.  It indexes Java class
and method declarations, Rust source modules, TODO/panic sites, and validates
the checked-in parity manifest.  It is a discovery tool, not proof that a
contract is complete; boundary/differential tests remain the acceptance gate.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

JAVA_CLASS = re.compile(r"\b(?:class|interface|enum|record)\s+(\w+)")
JAVA_METHOD = re.compile(r"(?:public|protected|private|static|final|synchronized|native|abstract|\s)+[\w<>\[\],.?]+\s+(\w+)\s*\(")
RUST_DECL = re.compile(r"\b(?:pub\s+)?(?:struct|enum|trait|fn|mod)\s+(\w+)")
MARKER = re.compile(r"\b(?:TODO|FIXME|todo!|unimplemented!|panic!)\b")


def files(root: Path, pattern: str) -> list[Path]:
    return sorted(p for p in root.rglob(pattern) if ".git" not in p.parts and "target" not in p.parts)


def declarations(paths: list[Path], pattern: re.Pattern[str], root: Path) -> dict[str, list[str]]:
    result: dict[str, list[str]] = {}
    for path in paths:
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        found = sorted(set(pattern.findall(text)))
        if found:
            result[str(path.relative_to(root))] = found
    return result


def load_manifest(root: Path) -> tuple[list[dict], list[str]]:
    """Parse the intentionally small TOML subset used by parity/manifest.toml."""
    manifest = root / "parity" / "manifest.toml"
    rows: list[dict] = []
    current: dict | None = None
    for raw in manifest.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line == "[[contract]]":
            current = {}
            rows.append(current)
            continue
        if current is None or "=" not in line:
            continue
        key, value = (part.strip() for part in line.split("=", 1))
        try:
            current[key] = json.loads(value)
        except json.JSONDecodeError:
            current[key] = value.strip('"')
    errors: list[str] = []
    required = {"id", "vanilla_version", "vanilla_sources", "pumpkin_sources", "status", "observable_contracts", "tests", "dependencies", "last_verified_commit"}
    allowed = {"complete", "mostly", "partial", "missing", "unknown", "not_applicable", "extension"}
    seen: set[str] = set()
    for row in rows:
        missing = required - row.keys()
        if missing:
            errors.append(f"{row.get('id', '<unknown>')}: missing {sorted(missing)}")
        if row.get("id") in seen:
            errors.append(f"duplicate contract id: {row['id']}")
        seen.add(row.get("id", ""))
        if row.get("status") not in allowed:
            errors.append(f"{row.get('id', '<unknown>')}: invalid status {row.get('status')!r}")
        if row.get("status") == "complete" and not row.get("tests"):
            errors.append(f"{row['id']}: complete contract has no tests")
        for source_key in ("vanilla_sources", "pumpkin_sources"):
            for source in row.get(source_key, []):
                if source_key == "pumpkin_sources" and not (root / source).exists():
                    errors.append(f"{row['id']}: missing Pumpkin source {source}")
                if source_key == "vanilla_sources" and not (root / "Minecraft" / "decompiled_src" / "sources" / source).exists():
                    # A checkout may intentionally omit the decompile; keep this
                    # as a warning in the report rather than a hard failure.
                    errors.append(f"{row['id']}: missing vanilla source {source}")
    return rows, errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    java = files(root / "Minecraft", "*.java")
    rust = files(root / "crates", "*.rs")
    rows, errors = load_manifest(root)
    markers = []
    for path in rust:
        text = path.read_text(encoding="utf-8", errors="replace")
        for line_no, line in enumerate(text.splitlines(), 1):
            if MARKER.search(line):
                markers.append({"file": str(path.relative_to(root)), "line": line_no, "text": line.strip()})
    report = {
        "version": "26.2",
        "java_files": len(java),
        "rust_files": len(rust),
        "java_classes": declarations(java, JAVA_CLASS, root),
        "java_methods": declarations(java, JAVA_METHOD, root),
        "rust_declarations": declarations(rust, RUST_DECL, root),
        "markers": markers,
        "contracts": rows,
        "manifest_errors": errors,
    }
    encoded = json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(encoded, encoding="utf-8")
    else:
        print(encoded, end="")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
