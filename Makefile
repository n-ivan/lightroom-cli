.PHONY: build check lua-check release

build:
	cargo build --release

check:
	cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && $(MAKE) lua-check

lua-check:
	for f in $$(find plugin -name '*.lua'); do luajit -b "$$f" /dev/null || exit 1; done && luajit plugin/tests/run.lua

release:
	scripts/build-release.sh
