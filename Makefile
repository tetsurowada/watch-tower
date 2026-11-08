# Build, bundle and install watch-tower. Everything except `test` runs on the macOS host.

APP_NAME    := WatchTower
BUNDLE_ID   := dev.watch-tower
LABEL       := dev.watch-tower
VERSION     := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

# Keep macOS build artifacts apart from the devcontainer's Linux ones.
TARGET_DIR  := target/macos
APP         := $(TARGET_DIR)/$(APP_NAME).app

INSTALL_DIR ?= $(HOME)/Applications
ADDR        ?= :4567
DOCKER      ?= $(shell command -v docker)
PLIST       := $(HOME)/Library/LaunchAgents/$(LABEL).plist
LOG         := $(HOME)/Library/Logs/watch-tower.log
DOMAIN       = gui/$(shell id -u)

.PHONY: test build bundle install uninstall restart logs

test:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

build:
	CARGO_TARGET_DIR=$(TARGET_DIR) cargo build --release

bundle: build
	rm -rf "$(APP)"
	mkdir -p "$(APP)/Contents/MacOS" "$(APP)/Contents/Resources"
	cp "$(TARGET_DIR)/release/watch-tower" "$(APP)/Contents/MacOS/watch-tower"
	sed -e 's|@BUNDLE_ID@|$(BUNDLE_ID)|' -e 's|@VERSION@|$(VERSION)|' \
		macos/Info.plist.in > "$(APP)/Contents/Info.plist"
	iconutil -c icns assets/AppIcon.iconset -o "$(APP)/Contents/Resources/AppIcon.icns"
	cp assets/state-*.png "$(APP)/Contents/Resources/"
	cp /System/Library/Sounds/Glass.aiff /System/Library/Sounds/Ping.aiff "$(APP)/Contents/Resources/"
	codesign --force --sign - "$(APP)"
	codesign --verify "$(APP)"

install: bundle
	@test -n "$(DOCKER)" || { echo "docker not found; run make install DOCKER=/path/to/docker"; exit 1; }
	mkdir -p "$(INSTALL_DIR)" "$(dir $(PLIST))" "$(dir $(LOG))"
	-launchctl bootout $(DOMAIN)/$(LABEL) 2>/dev/null
	rm -rf "$(INSTALL_DIR)/$(APP_NAME).app"
	cp -R "$(APP)" "$(INSTALL_DIR)/"
	sed -e 's|@LABEL@|$(LABEL)|' \
		-e 's|@BIN@|$(INSTALL_DIR)/$(APP_NAME).app/Contents/MacOS/watch-tower|' \
		-e 's|@ADDR@|$(ADDR)|' \
		-e 's|@DOCKER@|$(DOCKER)|' \
		-e 's|@PATH@|$(dir $(DOCKER)):/usr/bin:/bin:/usr/sbin:/sbin|' \
		-e 's|@LOG@|$(LOG)|' \
		macos/launchd.plist.in > "$(PLIST)"
	launchctl bootstrap $(DOMAIN) "$(PLIST)"
	@echo "installed $(INSTALL_DIR)/$(APP_NAME).app; log: $(LOG)"
	@echo "first install: turn on notifications for watch-tower in System Settings > Notifications"

uninstall:
	-launchctl bootout $(DOMAIN)/$(LABEL) 2>/dev/null
	rm -f "$(PLIST)"
	rm -rf "$(INSTALL_DIR)/$(APP_NAME).app"

restart:
	launchctl kickstart -k $(DOMAIN)/$(LABEL)

logs:
	tail -f "$(LOG)"
