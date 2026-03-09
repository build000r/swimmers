SHELL := /bin/bash
.DEFAULT_GOAL := help

.PHONY: help server tui tui-check

help:
	@printf '%s\n' \
	'throngterm commands' \
	'' \
	'  make server                  Run the Rust server on the configured port' \
	'  make tui                     Wait for the API, then launch the native TUI' \
	'  make tui-check              Wait for the API and exit without launching the TUI'

server:
	cargo run --bin throngterm

tui:
	bash ./scripts/run-tui.sh

tui-check:
	TUI_WAIT_ONLY=1 bash ./scripts/run-tui.sh
