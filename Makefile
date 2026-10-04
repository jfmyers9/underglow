.PHONY: check fmt clippy test run-info run-test run-effect run-command-pulse config-dry-run command-pulse-dry-run github-ci-dry-run focus-dry-run market-dry-run sports-dry-run app-aura-dry-run soundwave-dry-run install-dry-run uninstall-dry-run

check: fmt clippy test

fmt:
	cargo fmt --check

clippy:
	cargo clippy --all-targets -- -D warnings

test:
	cargo test

run-info:
	cargo run -- info

run-test:
	cargo run -- test --brightness 96 --seconds 3

run-effect:
	cargo run -- effect comet --palette cyberpunk --brightness 128 --seconds 10 --fps 30

run-command-pulse:
	cargo run -- signal run command-pulse -- make check

config-dry-run:
	cargo run -- run --config examples/wooting-signals.toml --dry-run

command-pulse-dry-run:
	cargo run -- run --config examples/command-pulse.toml --dry-run

github-ci-dry-run:
	cargo run -- run --config examples/github-ci.toml --dry-run

focus-dry-run:
	cargo run -- run --config examples/focus-cockpit.toml --dry-run

market-dry-run:
	cargo run -- run --config examples/market-pulse.toml --dry-run

sports-dry-run:
	cargo run -- run --config examples/sports-alerts.toml --dry-run

app-aura-dry-run:
	cargo run -- run --config examples/app-aura.toml --dry-run

soundwave-dry-run:
	cargo run -- run --config examples/soundwave.toml --dry-run

install-dry-run:
	@test -n "$(PACKAGE)" || { echo 'Use make install-dry-run PACKAGE=/path/to/extracted/release' >&2; exit 2; }
	@if [ "$$(uname -s)" = Darwin ]; then scripts/install-macos.sh --package "$(PACKAGE)"; else scripts/install-linux.sh --package "$(PACKAGE)"; fi

uninstall-dry-run:
	@if [ "$$(uname -s)" = Darwin ]; then scripts/uninstall-macos.sh; else scripts/uninstall-linux.sh; fi

.PHONY: dev test-dev
dev:
	python3 scripts/dev.py $(if $(filter 1,$(DEV_HARDWARE)),--hardware,)

test-dev:
	python3 -m unittest discover -s scripts/tests -p 'test_dev.py'
