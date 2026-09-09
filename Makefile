BIN := target/release/vitals
PREFIX ?= /usr/local

.PHONY: build test install uninstall fmt lint dashboard bundle install-app uninstall-app

# The React dashboard `crates/app/src/serve/assets.rs` embeds via
# `include_dir!`. That macro only needs `dashboard/dist` to exist (a fresh
# clone has it, as a placeholder holding just `.gitkeep` — see
# `.gitignore`), so `cargo check`/`cargo build` work without this target.
# But a *useful* binary needs the real build output in there, so both
# `build` and `test` depend on it: `cargo test --workspace` alone, without
# ever running this, will fail the dashboard-asset integration test in
# `crates/app/tests/serve.rs` for the same reason — no built assets to serve.
dashboard:
	cd dashboard && npm ci && npm run build
# `vite build` empties outDir, which deletes the committed
# dashboard/dist/.gitkeep placeholder. That placeholder is what lets a fresh
# clone compile at all -- include_dir! fails the build outright if
# dashboard/dist does not exist -- so losing it leaves a dirty tree, and
# committing the deletion breaks `cargo build` for everyone who clones next.
	touch dashboard/dist/.gitkeep

build: dashboard
	cargo build --release

test: dashboard
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

# Packages target/release/vitals into dist/Vitals.app: an ad-hoc-signed,
# LSUIElement app bundle (no Dock icon, no app switcher entry). See
# scripts/bundle.sh for the signing rationale.
bundle: build
	./scripts/bundle.sh

# Installs the bundle to /Applications, registers the LaunchAgent so the
# tray starts at login, and symlinks $(PREFIX)/bin/vitals to the bundled
# binary (overwriting the `install` target's symlink to the raw build
# output, if that was used first -- both ultimately point at the same
# vitals binary, just reached through different paths).
install-app: bundle
	rm -rf /Applications/Vitals.app
	cp -R dist/Vitals.app /Applications/
	cp resources/com.billsun.vitals.plist ~/Library/LaunchAgents/
	launchctl unload ~/Library/LaunchAgents/com.billsun.vitals.plist 2>/dev/null || true
	launchctl load ~/Library/LaunchAgents/com.billsun.vitals.plist
	ln -sf /Applications/Vitals.app/Contents/MacOS/vitals $(PREFIX)/bin/vitals

uninstall-app:
	launchctl unload ~/Library/LaunchAgents/com.billsun.vitals.plist 2>/dev/null || true
	rm -f ~/Library/LaunchAgents/com.billsun.vitals.plist
	rm -rf /Applications/Vitals.app
	rm -f $(PREFIX)/bin/vitals
