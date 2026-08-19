#!/usr/bin/env python3
"""Deterministic, dependency-free runner for Pumpkin/vanilla replay scenarios.

The runner intentionally keeps the server adapter small and explicit.  A
scenario is executed through RCON on each server, while the adapter records
commands, tick barriers and server replies.  Packet bots and world snapshot
collectors can append records through the same JSONL trace format; this keeps
the comparison/canonicalisation layer useful before a particular protocol bot
is available for a new Minecraft protocol revision.

The wire implementation follows the Source RCON packet format used by the
vanilla dedicated server.  No third-party Python package is required, which
allows this tool to run in CI and in a clean checkout.
"""

from __future__ import annotations

import argparse
import difflib
import json
import re
import shlex
import socket
import struct
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable


VOLATILE_KEYS = {
    "timestamp",
    "wall_time",
    "session_id",
    "compression_threshold",
    "keep_alive_id",
}
UUID_RE = re.compile(
    r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b"
)


def _read_exact(stream: socket.socket, size: int) -> bytes:
    chunks: list[bytes] = []
    remaining = size
    while remaining:
        chunk = stream.recv(remaining)
        if not chunk:
            raise ConnectionError("RCON connection closed while reading a packet")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


class RconClient:
    """Minimal authenticated RCON client with strict packet validation."""

    def __init__(self, host: str, port: int, password: str, timeout: float = 5.0):
        self.host = host
        self.port = port
        self.password = password
        self.timeout = timeout
        self._socket: socket.socket | None = None
        self._request_id = 0

    def connect(self) -> None:
        if self._socket is not None:
            return
        sock = socket.create_connection((self.host, self.port), self.timeout)
        sock.settimeout(self.timeout)
        self._socket = sock
        response_id, _response_type, _payload = self._request(3, self.password)
        if response_id == -1:
            self.close()
            raise PermissionError("RCON authentication failed")

    def close(self) -> None:
        if self._socket is not None:
            self._socket.close()
            self._socket = None

    def command(self, command: str) -> str:
        if self._socket is None:
            self.connect()
        _response_id, _response_type, payload = self._request(2, command)
        return payload

    def _request(self, packet_type: int, payload: str) -> tuple[int, int, str]:
        if self._socket is None:
            raise ConnectionError("RCON is not connected")
        self._request_id += 1
        request_id = self._request_id
        encoded = payload.encode("utf-8") + b"\x00\x00"
        body = struct.pack("<ii", request_id, packet_type) + encoded
        self._socket.sendall(struct.pack("<i", len(body)) + body)
        length = struct.unpack("<i", _read_exact(self._socket, 4))[0]
        if length < 10 or length > 16 * 1024 * 1024:
            raise ValueError(f"invalid RCON packet length {length}")
        packet = _read_exact(self._socket, length)
        response_id, response_type = struct.unpack("<ii", packet[:8])
        if packet[-2:] != b"\x00\x00":
            raise ValueError("RCON packet is missing its terminator")
        return response_id, response_type, packet[8:-2].decode("utf-8", "replace")


@dataclass(frozen=True)
class TraceRecord:
    server: str
    step: int
    kind: str
    value: Any

    def as_dict(self) -> dict[str, Any]:
        return {
            "server": self.server,
            "step": self.step,
            "kind": self.kind,
            "value": self.value,
        }


@dataclass
class ServerSpec:
    name: str
    command: list[str]
    cwd: Path | None
    rcon_host: str
    rcon_port: int
    rcon_password: str
    startup_timeout: float = 60.0


def canonicalize(value: Any, *, list_order_matters: bool = True) -> Any:
    """Canonicalise trace/NBT-like JSON without destroying list semantics."""

    if isinstance(value, dict):
        return {
            key: canonicalize(child, list_order_matters=list_order_matters)
            for key, child in sorted(value.items())
            if key not in VOLATILE_KEYS
        }
    if isinstance(value, list):
        items = [canonicalize(item, list_order_matters=list_order_matters) for item in value]
        return items if list_order_matters else sorted(items, key=lambda item: json.dumps(item, sort_keys=True))
    if isinstance(value, str):
        return UUID_RE.sub("<uuid>", value)
    return value


def canonical_jsonl(path: Path) -> list[str]:
    lines: list[str] = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        if not raw.strip():
            continue
        lines.append(json.dumps(canonicalize(json.loads(raw)), sort_keys=True, separators=(",", ":")))
    return lines


def write_record(path: Path, record: TraceRecord) -> None:
    with path.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(record.as_dict(), ensure_ascii=False, sort_keys=True) + "\n")


def _scalar(value: str) -> Any:
    value = value.strip()
    if not value:
        return ""
    if value in {"true", "false"}:
        return value == "true"
    if value in {"null", "~"}:
        return None
    try:
        return json.loads(value)
    except json.JSONDecodeError:
        return value.strip("'\"")


def load_scenario(path: Path) -> list[dict[str, Any]]:
    """Load JSON or the deliberately small scenario YAML subset.

    YAML scenarios are a list of maps with scalar values, for example::

        - command: "time query gametime"
        - wait_ticks: 5

    Complex values should use JSON; this avoids silently accepting ambiguous
    YAML features and keeps the runner dependency-free.
    """

    text = path.read_text(encoding="utf-8")
    try:
        parsed = json.loads(text)
        if not isinstance(parsed, list):
            raise ValueError("scenario JSON must be a list")
        return parsed
    except json.JSONDecodeError:
        pass

    result: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].rstrip()
        if not line.strip():
            continue
        stripped = line.strip()
        if stripped.startswith("-"):
            if current is not None:
                result.append(current)
            current = {}
            stripped = stripped[1:].strip()
            if stripped:
                if ":" not in stripped:
                    raise ValueError(f"{path}:{number}: expected key: value")
                key, value = stripped.split(":", 1)
                current[key.strip()] = _scalar(value)
            continue
        if current is None or ":" not in stripped:
            raise ValueError(f"{path}:{number}: expected a list item")
        key, value = stripped.split(":", 1)
        current[key.strip()] = _scalar(value)
    if current is not None:
        result.append(current)
    if not result:
        raise ValueError(f"{path}: scenario has no steps")
    return result


def wait_for_rcon(spec: ServerSpec) -> RconClient:
    deadline = time.monotonic() + spec.startup_timeout
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        client = RconClient(spec.rcon_host, spec.rcon_port, spec.rcon_password)
        try:
            client.connect()
            return client
        except (OSError, ConnectionError, PermissionError, ValueError) as error:
            last_error = error
            client.close()
            time.sleep(0.25)
    raise TimeoutError(f"RCON did not become ready: {last_error}")


def wait_ticks(client: RconClient, count: int, timeout: float = 60.0) -> int:
    if count < 0:
        raise ValueError("wait_ticks count must be non-negative")
    initial = int(client.command("time query gametime").strip().split()[-1])
    target = initial + count
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        current = int(client.command("time query gametime").strip().split()[-1])
        if current >= target:
            return current
        time.sleep(0.05)
    raise TimeoutError(f"server did not reach game time {target}")


def run_scenario(spec: ServerSpec, scenario: Iterable[dict[str, Any]], trace: Path) -> int:
    process: subprocess.Popen[bytes] | None = None
    if spec.command:
        process = subprocess.Popen(spec.command, cwd=spec.cwd, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    client = wait_for_rcon(spec)
    try:
        for step_number, step in enumerate(scenario):
            if "command" in step:
                command = str(step["command"])
                value = client.command(command)
                write_record(trace, TraceRecord(spec.name, step_number, "rcon", {"command": command, "reply": value}))
            elif "wait_ticks" in step:
                value = wait_ticks(client, int(step["wait_ticks"]))
                write_record(trace, TraceRecord(spec.name, step_number, "tick_barrier", value))
            elif "sleep" in step:
                time.sleep(float(step["sleep"]))
                write_record(trace, TraceRecord(spec.name, step_number, "sleep", float(step["sleep"])))
            else:
                raise ValueError(f"step {step_number} has no supported action: {step}")
    finally:
        client.close()
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
    return 0


def compare_traces(left: Path, right: Path) -> int:
    left_lines = canonical_jsonl(left)
    right_lines = canonical_jsonl(right)
    if left_lines == right_lines:
        print("traces match")
        return 0
    print("traces differ", file=sys.stderr)
    print("\n".join(difflib.unified_diff(left_lines, right_lines, fromfile=str(left), tofile=str(right))))
    return 1


def _server_spec(args: argparse.Namespace, name: str) -> ServerSpec:
    command = getattr(args, f"{name}_command")
    return ServerSpec(
        name=name,
        command=shlex.split(command) if command else [],
        cwd=Path(getattr(args, f"{name}_cwd")).resolve() if getattr(args, f"{name}_cwd") else None,
        rcon_host=args.rcon_host,
        rcon_port=getattr(args, f"{name}_rcon_port"),
        rcon_password=args.rcon_password,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)

    compare = subparsers.add_parser("compare", help="compare two JSONL traces")
    compare.add_argument("left", type=Path)
    compare.add_argument("right", type=Path)

    run = subparsers.add_parser("run", help="run a scenario against one or both servers")
    run.add_argument("scenario", type=Path)
    run.add_argument("--vanilla-command")
    run.add_argument("--pumpkin-command")
    run.add_argument("--vanilla-cwd")
    run.add_argument("--pumpkin-cwd")
    run.add_argument("--vanilla-rcon-port", type=int, default=25575)
    run.add_argument("--pumpkin-rcon-port", type=int, default=25576)
    run.add_argument("--rcon-host", default="127.0.0.1")
    run.add_argument("--rcon-password", required=True)
    run.add_argument("--output-dir", type=Path, default=Path("parity-traces"))

    args = parser.parse_args()
    if args.action == "compare":
        return compare_traces(args.left, args.right)
    scenario = load_scenario(args.scenario)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    ran = False
    for name in ("vanilla", "pumpkin"):
        if getattr(args, f"{name}_command"):
            ran = True
            trace = args.output_dir / f"{name}.jsonl"
            trace.unlink(missing_ok=True)
            run_scenario(_server_spec(args, name), scenario, trace)
    if not ran:
        raise SystemExit("provide --vanilla-command and/or --pumpkin-command")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
