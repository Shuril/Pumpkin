# Parity runner

`parity_runner.py` is a dependency-free replay and trace tool for comparing a
vanilla dedicated server with Pumpkin. It intentionally uses the same scenario
and command stream for both processes and records JSONL traces with explicit
tick barriers.

Run a scenario against both servers (the servers must expose RCON):

```bash
python3 tools/parity-runner/parity_runner.py run \
  tools/parity-runner/example.scenario.yaml \
  --vanilla-command 'java -jar server.jar nogui' \
  --vanilla-cwd /tmp/vanilla \
  --vanilla-rcon-port 25575 \
  --pumpkin-command 'cargo run --release --bin pumpkin -- --nogui' \
  --pumpkin-cwd /tmp/pumpkin \
  --pumpkin-rcon-port 25576 \
  --rcon-password change-me

python3 tools/parity-runner/parity_runner.py compare \
  parity-traces/vanilla.jsonl parity-traces/pumpkin.jsonl
```

The runner does not pretend to be a Java packet bot. A packet adapter can
append normalized records to the same JSONL format, while RCON remains useful
for command, save/reload, and tick-barrier scenarios. UUIDs and volatile
session fields are normalized; list order is retained because packet and tick
ordering is observable.
