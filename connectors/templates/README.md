# __SOURCE__ connector starter

This example reports a timed focus session, not keyboard colors. Edit the fixed
state/event labels and lifecycle to match your application. No package install,
keyboard SDK, network, shell execution, or engine changes are needed.

Python: `python3 connector.py --stdout --duration-seconds 10`

TypeScript (Node 22.19+): `node connector.ts --stdout --duration-seconds 10`

Only the file for your selected language is generated. Replace `--stdout` with
`--socket /actual/engine/state/events.sock` for direct publishing. Socket selection
is deliberately explicit; a starter never guesses which installed engine to use.
Each invocation uses a fresh random identity. State renews every five seconds with
a 15-second lease; normal completion emits `done`, then removes state. Ctrl-C or
SIGTERM attempts removal without emitting completion. Errors terminate promptly;
this small starter does not implement Pi's reconnect/backoff behavior.

If generated with `--capabilities`, `capabilities.toml` is inert metadata for
the GUI **Connections → Import custom capability manifest** workflow. It
declares `focusing` and `done`, not task success. Import it to configure a layer
with the generic behavior forms, without Rust changes. Update this manifest
alongside any fact/lifecycle changes to the producer. Import never installs,
trusts, enables or runs this program; review executable trust separately.

Keep stdout exclusively NDJSON in `--stdout` mode; diagnostics belong on stderr.
This mode emits the same messages for a supervised process reader, without socket
ACKs. A consumer must apply its own bounds and trust policy. No service is installed
or enabled by generating or running this example.

From the repository, check a fixture or captured output:

```sh
python3 scripts/check-connector.py /path/to/fixture.ndjson
```

Fixtures use a fixed identity **only for offline tests**. Never replay them against
your engine as a real session. Read `docs/connectors.md` in the repository for the
wire contract, rule configuration, validation limits, and security boundaries.
