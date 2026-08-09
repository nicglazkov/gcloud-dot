VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP     := build/GCloud Dot.app
DMG     := GCloud-Dot-$(VERSION).dmg

# Signing. Copy signing.env.example to signing.env and fill it in; that file is
# gitignored because a Team ID identifies whoever ships the app, not the project.
-include signing.env

# signing.env is also meant to be `source`-able from a shell, so its values are
# quoted. make keeps those quotes as part of the value, which then reach the
# shell doubled and break on the parentheses in a signing identity. Strip one
# layer here so the file stays valid for both readers.
# `patsubst` would be the obvious tool and is the wrong one: it splits on
# whitespace, and every value here contains spaces, so no word ever matches the
# whole quoted string. Removing the quote character outright is what works.
unquote = $(subst ",,$(1))
SIGN_IDENTITY := $(call unquote,$(SIGN_IDENTITY))
TEAM_ID       := $(call unquote,$(TEAM_ID))
ASC_KEY_PATH  := $(call unquote,$(ASC_KEY_PATH))
ASC_KEY_ID    := $(call unquote,$(ASC_KEY_ID))
ASC_ISSUER_ID := $(call unquote,$(ASC_ISSUER_ID))

SIGN_IDENTITY ?= -
TEAM_ID       ?=

.PHONY: all build test lint icons app sign notarize dmg dmg-background verify clean \
        linux-deb linux-appimage windows-zip dist-macos help

help:
	@echo "make test        run the whole test suite"
	@echo "make app         build a universal .app bundle"
	@echo "make dist-macos  build, sign, notarize, staple, and package a DMG"
	@echo "make verify      prove Gatekeeper would accept the artifacts, offline"

all: build

build:
	cargo build --release

test:
	cargo test --workspace

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

# ------------------------------------------------------------------- icons
# Regenerates every PNG the site and the bundle use, straight from the code
# that draws them at runtime, so documentation cannot drift from behaviour.
icons:
	cargo run -q -p gcloud-dot-app --example render_icons -- site/img
	rm -rf build/GCloudDot.iconset && mkdir -p build/GCloudDot.iconset
	for s in 16 32 64 128 256 512; do \
	  sips -z $$s $$s site/img/appicon.png --out build/GCloudDot.iconset/icon_$${s}x$${s}.png >/dev/null; \
	  d=$$((s * 2)); \
	  sips -z $$d $$d site/img/appicon.png --out build/GCloudDot.iconset/icon_$${s}x$${s}@2x.png >/dev/null; \
	done
	rm -f build/GCloudDot.iconset/icon_512x512@2x.png
	sips -z 1024 1024 site/img/appicon.png --out build/GCloudDot.iconset/icon_512x512@2x.png >/dev/null
	iconutil -c icns build/GCloudDot.iconset -o build/GCloudDot.icns
	@echo "wrote build/GCloudDot.icns"

# --------------------------------------------------------------- macOS app
# One universal binary rather than two builds: GitHub has retired its Intel
# macOS runners, so an x86_64 job queues forever, and lipo on an arm64 runner
# covers both architectures in a single asset.
app: icons
	rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true
	cargo build --release --target aarch64-apple-darwin
	cargo build --release --target x86_64-apple-darwin
	rm -rf "$(APP)"
	mkdir -p "$(APP)/Contents/MacOS" "$(APP)/Contents/Resources"
	lipo -create -output "$(APP)/Contents/MacOS/GCloudDot" \
	  target/aarch64-apple-darwin/release/gcloud-dot-tray \
	  target/x86_64-apple-darwin/release/gcloud-dot-tray
	lipo -create -output "$(APP)/Contents/MacOS/gcloud-dot" \
	  target/aarch64-apple-darwin/release/gcloud-dot \
	  target/x86_64-apple-darwin/release/gcloud-dot
	sed 's/__VERSION__/$(VERSION)/g' packaging/macos/Info.plist > "$(APP)/Contents/Info.plist"
	cp build/GCloudDot.icns "$(APP)/Contents/Resources/GCloudDot.icns"
	@echo "built $(APP) for $$(lipo -archs "$(APP)/Contents/MacOS/GCloudDot")"

# Hardened runtime is required for notarization. The CLI inside the bundle is
# signed first: codesign seals the bundle around whatever it finds, so signing
# the outer bundle before its nested executables produces a seal that no longer
# matches once they are signed.
sign: app
	codesign --force --options runtime --timestamp \
	  --entitlements packaging/macos/entitlements.plist \
	  -s "$(SIGN_IDENTITY)" "$(APP)/Contents/MacOS/gcloud-dot"
	codesign --force --options runtime --timestamp \
	  --entitlements packaging/macos/entitlements.plist \
	  -s "$(SIGN_IDENTITY)" "$(APP)/Contents/MacOS/GCloudDot"
	codesign --force --options runtime --timestamp \
	  --entitlements packaging/macos/entitlements.plist \
	  -s "$(SIGN_IDENTITY)" "$(APP)"
	codesign --verify --deep --strict --verbose=2 "$(APP)"

# Two submissions, deliberately.
#
# Notarizing only the DMG leaves the app inside it unstapled, because the DMG is
# assembled before the ticket exists and stapling a disk image does not staple
# what is sealed within it. Gatekeeper still passes such an app by asking Apple
# online, so the gap is invisible on a connected machine and blocks the user
# whose first launch is offline, on a plane, or behind a captive portal, which
# is exactly when someone reaches for a tool about expired credentials.
notarize: sign
	@test -n "$(ASC_KEY_PATH)" || { echo "set ASC_KEY_PATH, ASC_KEY_ID and ASC_ISSUER_ID in signing.env"; exit 1; }
	@# Preflight what notarization will reject, before spending a round trip.
	@set -e; INFO="$$(codesign -dv --verbose=4 "$(APP)" 2>&1)"; \
	 case "$$INFO" in *"flags=0x10000(runtime)"*) ;; \
	   *) echo "hardened runtime missing"; exit 1;; esac; \
	 case "$$INFO" in *"Authority=Developer ID Application"*) ;; \
	   *) echo "not signed with a Developer ID"; exit 1;; esac; \
	 echo "preflight ok"
	rm -f build/notarize.zip
	ditto -c -k --keepParent "$(APP)" build/notarize.zip
	xcrun notarytool submit build/notarize.zip \
	  --key "$(ASC_KEY_PATH)" --key-id "$(ASC_KEY_ID)" --issuer "$(ASC_ISSUER_ID)" --wait
	xcrun stapler staple "$(APP)"
	rm -f build/notarize.zip
	$(MAKE) dmg
	@# Signing the disk image is not optional the way it looks. Notarizing and
	@# stapling an unsigned image still leaves `spctl -a -t open` reporting
	@# "no usable signature": the ticket attests that the contents were checked,
	@# while nothing attests to who produced the container holding them.
	codesign --force --timestamp -s "$(SIGN_IDENTITY)" "$(DMG)"
	xcrun notarytool submit "$(DMG)" \
	  --key "$(ASC_KEY_PATH)" --key-id "$(ASC_KEY_ID)" --issuer "$(ASC_ISSUER_ID)" --wait
	xcrun stapler staple "$(DMG)"

# The installer window: a painted background, the app on the left, an alias to
# Applications on the right, and an arrow between them.
#
# dmgbuild rather than create-dmg. create-dmg drives Finder over AppleScript to
# place the icons, which needs a GUI session and an automation grant; dmgbuild
# writes the .DS_Store directly, so the layout is produced by code instead of by
# remote-controlling a window that may not exist.
dmg: dmg-background
	@test -d "$(APP)" || { echo "no app at $(APP); run make app first"; exit 1; }
	python3 packaging/macos/build-dmg.py "$(APP)" "$(DMG)" build/dmg-bg

# The background is drawn by the same code that draws the tray icons, at 1x and
# 2x, so it cannot drift from the product it is advertising.
dmg-background:
	@mkdir -p build
	cargo run -q -p gcloud-dot-app --example render_dmg_background -- build/dmg-bg

# Confirms Gatekeeper would accept these artifacts with the network unplugged.
# The whole release flow for macOS, in the only order that staples everything.
dist-macos: notarize verify

# Confirms Gatekeeper would accept both artifacts with the network unplugged.
verify:
	@echo "--- signature ---"
	@codesign -dv --verbose=2 "$(APP)" 2>&1 | grep -E "Authority|TeamIdentifier|flags" || true
	@echo "--- gatekeeper ---"
	@spctl -a -vvv -t install "$(APP)" || true
	@echo "--- ticket stapled to the app? ---"
	@xcrun stapler validate "$(APP)" || true
	@echo "--- ticket stapled to the dmg? ---"
	@test -f "$(DMG)" && xcrun stapler validate "$(DMG)" || echo "(no dmg built)"
	@echo "--- gatekeeper on the dmg, as a download ---"
	@test -f "$(DMG)" && spctl -a -vvv -t open --context context:primary-signature "$(DMG)" || true
	@echo "--- architectures ---"
	@lipo -archs "$(APP)/Contents/MacOS/GCloudDot" || true

clean:
	cargo clean
	rm -rf build *.dmg *.zip
