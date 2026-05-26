# xerotop — convenience wrapper around cargo.
# The release binary lives at target/release/xerotop; `make install` symlinks it
# onto your PATH (~/.local/bin) so `xerotop` and the labwc menu pick up rebuilds.

PREFIX  ?= $(HOME)/.local
BINDIR  ?= $(PREFIX)/bin
BIN      = target/release/xerotop

# Static landing page (www/) -> the kabyhills box, served at xerotop.com via Cloudflare.
SITE_HOST ?= kh
SITE_ROOT ?= /srv/www/xerotop

.PHONY: build run restart install uninstall debug check fmt clean deploy-site

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

# Symlink the release binary onto PATH (idempotent).
install: build
	mkdir -p $(BINDIR)
	ln -sf $(CURDIR)/$(BIN) $(BINDIR)/xerotop
	@echo "linked $(BINDIR)/xerotop -> $(CURDIR)/$(BIN)"

uninstall:
	rm -f $(BINDIR)/xerotop

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
