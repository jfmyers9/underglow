"""Drive the production supervisor, replacing only compilation and GUI binaries."""
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys

repository, checkout, binary, existing = sys.argv[1:]
repository, checkout = Path(repository), Path(checkout)
spec = importlib.util.spec_from_file_location("dev", repository / "scripts/dev.py")
dev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dev)


class Supervisor(dev.Supervisor):
    def build(self):
        return True

    def ready(self):
        # Discovery must not advertise an engine that is still starting.
        assert not self.discovery.exists()
        super().ready()


def reply(value):
    print(json.dumps(value), flush=True)


os.umask(0o077)
(checkout / "target/debug").mkdir(parents=True)
(checkout / "examples").mkdir()
shutil.copy2(binary, checkout / "target/debug/underglow")
shutil.copy2(repository / "examples/matrix-pi.toml", checkout / "examples/matrix-pi.toml")
for name in ("underglow-gui", "underglow-service"):
    executable = checkout / "target/debug" / name
    executable.write_text("#!/bin/sh\nexec sleep 120\n")
    executable.chmod(0o700)

runtime = dev.runtime_path(checkout)
supervisor = Supervisor(checkout, runtime)
if existing == "existing":
    # A previous saved selection without Pi rules; startup must merge, not reset.
    (supervisor.state / "state.json").write_text(json.dumps({
        "schema_version": 1, "enabled": False,
        "config": "effect='breath'\nbrightness=73\nspeed=65\nfps=30\n",
    }))
lock = dev.acquire_lock(runtime)
current = {"src/main.rs": b"initial"}
try:
    with contextlib.redirect_stdout(sys.stderr):
        assert supervisor.rebuild(current)
    reply({"runtime": str(runtime), "descriptor": json.loads(supervisor.discovery.read_text())})
    for line in sys.stdin:
        action = json.loads(line)["action"]
        if action == "close":
            supervisor.close()
            reply({"closed": True, "discovery_exists": supervisor.discovery.exists()})
        else:
            key = "src/bin/gui/preview.rs" if action == "gui" else "src/main.rs"
            current = dict(current, **{key: os.urandom(8)})
            with contextlib.redirect_stdout(sys.stderr):
                assert supervisor.rebuild(current)
            reply({"descriptor": json.loads(supervisor.discovery.read_text())})
finally:
    supervisor.close()
    lock.close()
    shutil.rmtree(runtime)
