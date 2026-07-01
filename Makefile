# xerotop — convenience wrapper around cargo.
# `make install` does a normal DESTDIR-aware install (binary + desktop + icon +
# license); `make link` is the dev shortcut that symlinks the release binary
# onto ~/.local/bin so a plain rebuild updates the running `xerotop`.

# Install prefix. Override for packaging: `make install DESTDIR="$$pkgdir" PREFIX=/usr`.
PREFIX  ?= /usr/local
DESTDIR ?=
BIN      = target/release/xerotop

# Static landing page (www/) -> the kabyhills box, served at xerotop.com via Cloudflare.
SITE_HOST ?= kh
SITE_ROOT ?= /srv/www/xerotop

.PHONY: build run restart install uninstall link unlink debug check fmt clean deploy-site

# Default: optimized build. The symlink means this alone updates `xerotop`.
build:
	cargo build --release

# Build, then launch in the foreground.
run: build
	./$(BIN)

# Rebuild and hot-swap the running bar (same as the labwc "XeroTop" menu item).
restart: build
	-pkill -x xerotop
	@sleep 0.4
	setsid ./$(BIN) >/dev/null 2>&1 < /dev/null &
	@echo "xerotop restarted"

# Normal install: copy the binary, desktop entry, icon and license under
# $(DESTDIR)$(PREFIX). DESTDIR-aware so packaging can stage it (the AUR PKGBUILD
# calls this from package()).
install: build
	install -Dm755 $(BIN) $(DESTDIR)$(PREFIX)/bin/xerotop
	install -Dm644 assets/cc.xeron.xerotop.desktop $(DESTDIR)$(PREFIX)/share/applications/cc.xeron.xerotop.desktop
	install -Dm644 assets/cc.xeron.xerotop.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/cc.xeron.xerotop.svg
	install -Dm644 LICENSE $(DESTDIR)$(PREFIX)/share/licenses/xerotop/LICENSE

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/xerotop
	rm -f $(DESTDIR)$(PREFIX)/share/applications/cc.xeron.xerotop.desktop
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/cc.xeron.xerotop.svg
	rm -rf $(DESTDIR)$(PREFIX)/share/licenses/xerotop

# Dev shortcut: symlink the release binary onto ~/.local/bin, so a plain
# `make build` / `cargo build` updates the running `xerotop` with no reinstall.
link: build
	mkdir -p $(HOME)/.local/bin
	ln -sf $(CURDIR)/$(BIN) $(HOME)/.local/bin/xerotop
	@echo "linked ~/.local/bin/xerotop -> $(CURDIR)/$(BIN)"

unlink:
	rm -f $(HOME)/.local/bin/xerotop

# Fast unoptimized build for iterating on logic.
debug:
	cargo build

check:
	cargo check

fmt:
	cargo fmt

clean:
	cargo clean

# Mirror www/ to the server (--delete so renamed/removed assets don't linger)
# and restore SELinux labels so freshly-rsynced images don't 403.
deploy-site:
	rsync -avz --delete --rsync-path="sudo rsync" www/ $(SITE_HOST):$(SITE_ROOT)/
	ssh $(SITE_HOST) 'sudo restorecon -Rv $(SITE_ROOT)/'
	@echo "deployed www/ -> $(SITE_HOST):$(SITE_ROOT)  (https://xerotop.com)"
