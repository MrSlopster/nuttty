# Quality pipeline for nuttty. `make check` is the full gate.

.PHONY: check fmt fmt-check lint test build clean

check: fmt-check lint test build

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

lint:
	cargo clippy --all-targets -- -D warnings

test:
	cargo test

build:
	cargo build --release

clean:
	cargo clean
