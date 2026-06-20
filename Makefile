SHELL := /bin/bash
.DEFAULT_GOAL := help

.PHONY: help server up tailnet kill up-smoke web web-smoke web-workbench-smoke tui tui-check tui-smoke tui-stress glance-smoke remote-auth-smoke multi-env-smoke multi-ssh-env-smoke multi-ssh-env-live-dry-run remote-rust-validate remote-rust-validate-dry-run release-acceptance release-acceptance-default release-acceptance-source release-acceptance-native release-acceptance-thought release-acceptance-voice release-acceptance-all ci-perf-gates cargo-cov-lcov

help:
	@printf '%s\n' \
	'swimmers commands' \
	'' \
	'  make server                  Run the Rust server on the configured port' \
	'  make up                      Start current-checkout backend, then launch web URLs + TUI' \
	'  make tailnet                 Run server on this machine'\''s Tailscale IP' \
	'  make kill                    Stop the swimmers backend listening on PORT (default 3210)' \
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
	'  make multi-env-smoke        Verify configured local+remote cockpit fixture contracts' \
	'  make multi-ssh-env-smoke    Verify v2 local+API+SSH-only fixture contracts' \
	'  make multi-ssh-env-live-dry-run Print the opt-in live target proof plan' \
	'  make remote-rust-validate-dry-run Print optional remote Cargo validation plan' \
	'  make remote-rust-validate  Run Rust validation on SWIMMERS_REMOTE_RUST_HOST' \
	'  make release-acceptance     Verify default installed-binary release smoke' \
	'  make release-acceptance-all Run default, source, native asset, and thought profiles' \
	'  make release-acceptance-voice Run optional voice-feature acceptance profile' \
	'  make ci-perf-gates          Run perf/concurrency gates (the CI regression guard bundle)' \
	'  make cargo-cov-lcov         Run Rust tests with lcov output for /crap'

server:
	cargo run --bin swimmers

up:
	bash ./scripts/run-up.sh

tailnet:
	bash ./scripts/run-tailnet.sh

kill:
	bash ./scripts/run-kill.sh

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

multi-env-smoke:
	bash ./scripts/test-multi-env-cockpit.sh

multi-ssh-env-smoke:
	bash ./scripts/test-multi-ssh-env-v2.sh

multi-ssh-env-live-dry-run:
	bash ./scripts/test-multi-ssh-env-live.sh --dry-run

remote-rust-validate:
	bash ./scripts/remote-rust-validate.sh

remote-rust-validate-dry-run:
	bash ./scripts/remote-rust-validate.sh --dry-run

release-acceptance: release-acceptance-default

release-acceptance-default:
	bash ./scripts/release-acceptance-smoke.sh default-installed

release-acceptance-source:
	bash ./scripts/release-acceptance-smoke.sh source-personal

release-acceptance-native:
	bash ./scripts/release-acceptance-smoke.sh native-assets

release-acceptance-thought:
	bash ./scripts/release-acceptance-smoke.sh thought

release-acceptance-voice:
	bash ./scripts/release-acceptance-smoke.sh voice

release-acceptance-all:
	bash ./scripts/release-acceptance-smoke.sh all

ci-perf-gates:
	bash ./scripts/ci-perf-gates.sh

cargo-cov-lcov:
	@llvm_cov="$${LLVM_COV:-$$(command -v llvm-cov || true)}"; \
	llvm_profdata="$${LLVM_PROFDATA:-$$(command -v llvm-profdata || true)}"; \
	cargo_target_dir="$${CARGO_TARGET_DIR:-}"; \
	cargo_llvm_cov_flags=""; \
	zig_bin="$${ZIG:-$$(command -v zig || true)}"; \
	if [ -z "$$cargo_target_dir" ] && [ -d target ] && [ ! -w target ]; then cargo_target_dir="/tmp/swimmers-llvm-cov-target"; cargo_llvm_cov_flags="--no-clean"; fi; \
	if [ -z "$${CC:-}" ] && [ -n "$$zig_bin" ]; then export CC="$(CURDIR)/scripts/zig-cc-rust-coverage.sh"; if [ -z "$${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-}" ]; then export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$(CURDIR)/scripts/zig-cc-rust-coverage.sh"; fi; fi; \
	if [ -n "$$llvm_cov" ]; then export LLVM_COV="$$llvm_cov"; fi; \
	if [ -n "$$llvm_profdata" ]; then export LLVM_PROFDATA="$$llvm_profdata"; fi; \
	if [ -n "$$cargo_target_dir" ]; then export CARGO_TARGET_DIR="$$cargo_target_dir"; fi; \
	cargo llvm-cov $$cargo_llvm_cov_flags --lcov --output-path lcov.info -- --test-threads=1
