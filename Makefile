BIN := target/release/vitals
PREFIX ?= /usr/local

.PHONY: build test install uninstall fmt lint

build:
	cargo build --release

test:
	cargo test --workspace

fmt:
	cargo fmt --all

lint:
	cargo clippy --workspace --all-targets -- -D warnings

install: build
	install -d $(PREFIX)/bin
	ln -sf $(CURDIR)/$(BIN) $(PREFIX)/bin/vitals
	@echo "installed $(PREFIX)/bin/vitals -> $(CURDIR)/$(BIN)"

uninstall:
	rm -f $(PREFIX)/bin/vitals
