SHELL := /bin/bash
.DEFAULT_GOAL := help

.PHONY: help server up up-smoke web web-smoke web-workbench-smoke tui tui-check tui-smoke tui-stress glance-smoke remote-auth-smoke ci-perf-gates cargo-cov-lcov

help:
	@printf '%s\n' \
	'swimmers commands' \
	'' \
	'  make server                  Run the Rust server on the configured port' \
	'  make up                      Start current-checkout backend, then launch web URLs + TUI' \
	'  make up-smoke                Run shell checks for the combined web+TUI launcher' \
	'  make web                     Run the server and print the local/tailnet browser URL' \
	'  make web-smoke              Verify live browser terminal attach and visible browser output' \
	'  make web-workbench-smoke    Verify the single-session workbench layout and widgets' \
	'  make tui                     Clear stale local API, then launch the native TUI' \
	'  make tui-check              Type-check the native TUI binary' \
	'  make tui-smoke              Run shell checks for the TUI bootstrap helper' \
	'  make tui-stress             Concurrent-load regression smoke for /v1/dirs + POST /v1/sessions' \
	'  make glance-smoke           Render the 10-session Glance fixture and write proof artifacts' \
	'  make remote-auth-smoke      Verify fake remote API launch/read auth and redaction paths' \
	'  make ci-perf-gates          Run perf/concurrency gates (the CI regression guard bundle)' \
	'  make cargo-cov-lcov         Run Rust tests with lcov output for /crap'

server:
	cargo run --bin swimmers

up:
	bash ./scripts/run-up.sh

up-smoke:
	bash ./scripts/test-run-up.sh

web:
	bash ./scripts/run-web.sh

web-smoke:
	PORT=3322 bash ./scripts/test-web-live-terminal.sh
	PORT=3323 bash ./scripts/test-web-visible-terminal.sh

web-workbench-smoke:
	PORT=3331 bash ./scripts/test-web-workbench.sh

tui:
	bash ./scripts/run-tui.sh

tui-check:
	cargo check --bin swimmers-tui

tui-smoke:
	bash ./scripts/test-run-tui.sh

tui-stress:
	bash ./scripts/stress-dirs-concurrency.sh

glance-smoke:
	bash ./scripts/test-glance-live.sh

remote-auth-smoke:
	bash ./scripts/test-remote-auth-smoke.sh

ci-perf-gates:
	bash ./scripts/ci-perf-gates.sh

cargo-cov-lcov:
	@llvm_cov="$${LLVM_COV:-$$(command -v llvm-cov || xcrun --find llvm-cov)}"; \
	llvm_profdata="$${LLVM_PROFDATA:-$$(command -v llvm-profdata || xcrun --find llvm-profdata)}"; \
	LLVM_COV="$$llvm_cov" LLVM_PROFDATA="$$llvm_profdata" cargo llvm-cov --lcov --output-path lcov.info
