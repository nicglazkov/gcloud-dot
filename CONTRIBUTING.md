# Contributing

Bug reports and pull requests are welcome.

## Getting set up

```sh
git clone https://github.com/nicglazkov/gcloud-dot.git
cd gcloud-dot
cargo test --workspace
```

Rust 1.89 or later, the single-instance lock uses `File::try_lock` from the
standard library, which stabilised there.

On Linux the tray needs system libraries:

```sh
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev libxdo-dev libdbus-1-dev
```

The `gcloud-dot` command needs none of them. Keeping it that way matters, CI builds it on a machine with no desktop libraries specifically to catch a GUI
dependency leaking into the core.

## Layout

```
crates/core   the engine: probing, log scanning, learning, state, settings
crates/cli    the gcloud-dot command
crates/app    the tray, the menu, and the details window
```

`core` owns every decision and knows nothing about windows, timers, or
processes. The engine says *what* should happen next and interprets what came
back; `app` does the blocking work on a worker thread. That split is what lets
the whole behaviour, including a session expiring while the machine was asleep, be tested in microseconds.

**New logic belongs in `core`, with a test.** If something can only be verified
by launching the app and looking at it, it is in the wrong crate.

## Tests

```sh
cargo test --workspace          # everything, headless, on any platform
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Every test runs without a window, a subprocess, or a network. Tests that need a
directory tree build one with `tempfile`; tests that need a moment in time
compute it from `Local::now()` rather than sleeping.

Look at the icons rather than only asserting about them:

```sh
cargo run -p gcloud-dot-app --example render_icons -- /tmp/icons
```

That is also how `site/img` is regenerated, so the website cannot drift from
what the app actually draws.

## Things worth knowing before changing the engine

- **Red must stay measured.** A probe failure that is not a recognised reauth
  error is `Indeterminate` and holds the previous verdict. Widening the error
  list to catch network failures would make the dot lie.
- **A session's end is timestamped at the midpoint** between the last good
  probe and the first bad one. Timestamping the failure overstates every
  session by one polling interval.
- **A login only counts if it completed.** Both the invocation marker and a
  completion marker must appear in gcloud's log, or an abandoned login is
  recorded as a session start and poisons the fallback estimator.
- **Every number shown must carry its provenance.** `EstimateSource` exists so
  the interface can never present a guess as a measurement.

## Style

Match what is there. Comments explain *why* a thing is the way it is, especially
where the obvious approach is wrong, those comments are load-bearing and
several of them record a bug that was actually hit.

## Releasing

Bump `version` in the workspace `Cargo.toml`, tag `vX.Y.Z`, and push the tag.
CI checks that the tag matches the crate version, builds every platform,
generates checksums, and publishes the release.

**macOS is signed on a Mac, not in CI.** Exporting a Developer ID certificate
requires a graphical confirmation that a headless runner cannot give, so rather
than keep a certificate in Actions secrets, the disk image is built locally:

```sh
python3 -m pip install --user dmgbuild   # builds the installer window
cp signing.env.example signing.env       # fill in your identity and API key
make dist-macos                          # sign, notarize, staple, package, verify
gh release upload vX.Y.Z GCloud-Dot-X.Y.Z.dmg
```

The installer window is laid out by `dmgbuild`, which writes the `.DS_Store`
directly. `create-dmg` drives Finder over AppleScript to place the icons, which
needs a GUI session and an automation grant, so it cannot run unattended. The
background is drawn by `make dmg-background`, from the same code that draws the
tray icons.

CI still builds and packages the macOS app on every release, so a packaging
break is caught at the normal time, it simply deletes the unsigned disk image
instead of publishing it. An unsigned DMG that looks official is worse than no
DMG, because it teaches people to click through the one warning that is meant
to mean something.

If you do configure `MACOS_CERT_P12`, `MACOS_CERT_PASSWORD`,
`MACOS_SIGN_IDENTITY`, `ASC_KEY_P8`, `ASC_KEY_ID`, and `ASC_ISSUER_ID`, the
release workflow notarizes and publishes the DMG on its own.
