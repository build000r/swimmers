SHELL := /bin/bash
.DEFAULT_GOAL := help

.PHONY: help server web web-smoke tui tui-check tui-smoke cargo-cov-lcov

help:
	@printf '%s\n' \
	'swimmers commands' \
	'' \
	'  make server                  Run the Rust server on the configured port' \
	'  make web                     Run the server and print the local/tailnet browser URL' \
	'  make web-smoke              Verify live browser terminal attach on a fresh session' \
	'  make tui                     Start a local API if needed, then launch the native TUI' \
	'  make tui-check              Wait for an existing API and exit without launching the TUI' \
	'  make tui-smoke              Run shell checks for the TUI bootstrap helper' \
	'  make cargo-cov-lcov         Run Rust tests with lcov output for /crap'

server:
	cargo run --bin swimmers

web:
	bash ./scripts/run-web.sh

web-smoke:
	bash ./scripts/test-web-live-terminal.sh

tui:
	bash ./scripts/run-tui.sh

tui-check:
	TUI_WAIT_ONLY=1 bash ./scripts/run-tui.sh

tui-smoke:
	bash ./scripts/test-run-tui.sh

cargo-cov-lcov:
	@llvm_cov="$${LLVM_COV:-$$(command -v llvm-cov || xcrun --find llvm-cov)}"; \
	llvm_profdata="$${LLVM_PROFDATA:-$$(command -v llvm-profdata || xcrun --find llvm-profdata)}"; \
	LLVM_COV="$$llvm_cov" LLVM_PROFDATA="$$llvm_profdata" cargo llvm-cov --lcov --output-path lcov.info
