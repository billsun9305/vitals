BIN := target/release/vitals
# Where the `vitals` symlink goes. $(HOME)/.local/bin needs no sudo;
# `make install PREFIX=/usr/local` if you'd rather have it there.
PREFIX ?= $(HOME)/.local
APP := /Applications/Vitals.app
# Left behind by installs older than the login-item registration.
LAUNCH_AGENT := $(HOME)/Library/LaunchAgents/com.billsun.vitals.plist

.PHONY: build test test-perf install uninstall fmt lint dashboard bundle install-app uninstall-app

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

# The two tests that actually protect the idle budget are #[ignore]d, because
# each takes ~16s of wall clock deliberately spent doing nothing. That means
# `make test` never runs them, so they are here instead -- run this before
# changing anything in cadence.rs or the sampler worker. They live in
# separate test binaries on purpose: park_cost measures RUSAGE_SELF, which
# sums every thread in the process, so a concurrent cadence test would
# pollute it.
test-perf:
	cargo test -p vitals-core --test park_cost -- --ignored --nocapture
	cargo test -p vitals-core --test cadence_period -- --ignored --nocapture

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

# Packages target/release/vitals into dist/Vitals.app: an LSUIElement app
# bundle (no Dock icon, no app switcher entry), ad-hoc signed unless
# SIGN_IDENTITY names a codesign identity. See scripts/bundle.sh.
bundle: build
	./scripts/bundle.sh

# Installs the bundle to /Applications, symlinks $(PREFIX)/bin/vitals to
# the bundled binary, and opens the app, which registers itself as a login
# item on its first bundled launch (crates/app/src/tray/login_item.rs). A
# running tray is quit first, or `open` would only re-front the old one. A
# LaunchAgent left by an older install is unloaded and removed so the tray
# is not started twice at login.
install-app: bundle
	install -d "$(PREFIX)/bin"
	-killall vitals 2>/dev/null
	rm -rf "$(APP)"
	cp -R dist/Vitals.app /Applications/
	ln -sf "$(APP)/Contents/MacOS/vitals" "$(PREFIX)/bin/vitals"
	if [ -f "$(LAUNCH_AGENT)" ]; then launchctl unload "$(LAUNCH_AGENT)" 2>/dev/null; rm -f "$(LAUNCH_AGENT)"; fi
	open "$(APP)"
	@echo "installed $(APP); $(PREFIX)/bin/vitals -> $(APP)/Contents/MacOS/vitals"

# Turns the login item off from the installed bundle (the registration
# names that bundle, so its own binary must do it), quits the tray, and
# removes the bundle and the symlink.
uninstall-app:
	-"$(APP)/Contents/MacOS/vitals" login-item off 2>/dev/null
	-killall vitals 2>/dev/null
	rm -rf "$(APP)"
	rm -f "$(PREFIX)/bin/vitals"
