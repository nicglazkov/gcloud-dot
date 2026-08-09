# GCloud Dot, design notes

Written 8 August 2026, for version 1.0.0. This records what the app does and,
more usefully, why several obvious approaches are wrong.

## The problem

Google enforces a reauth session on `gcloud` credentials, sixteen hours by
default on Workspace accounts, and exposes no deadline to the client. The
session length is enforced server side and never handed over. There is nothing
to read, so a client that wants to warn you has to infer it.

Two predecessors informed this design: a Swift menu bar app installed by a shell
script on macOS, and a PowerShell tray app on Windows. Both worked. The Windows
one was substantially more capable, and its state file, eighteen observed
sessions, every one between 15.91h and 16.08h, is the evidence that the
learning approach converges.

## The governing distinction

The app reports two things that must never blur together:

| | Source | Shown as |
|---|---|---|
| **Is the credential alive?** | Measured, by running a real command | Red, green, grey |
| **How long does it have?** | Predicted, from sessions observed ending | A countdown, labelled with its evidence |

Every number in the interface carries its provenance (`EstimateSource`), and the
one JSON field most likely to be consumed by a script is
`estimate_is_measured`.

The corollary that keeps the app trustworthy: a probe failure that is *not* a
recognised reauth error is `Indeterminate`, not expiry. It holds the previous
verdict and annotates the tooltip. Widening the reauth marker list to catch
network failures would turn the dot red on a captive portal, and a dot that
cries wolf is a dot people stop believing.

## Architecture

```
crates/core   engine, no threads, no timers, no windows
crates/cli    gcloud-dot, no GUI dependency, builds headless
crates/app    tray (tray-icon + muda), panel (wry), notifications
```

`core::engine::Engine` is a state machine with no I/O. `plan(now)` says what is
due; `apply_user_probe`, `apply_adc_probe`, and `apply_logins` interpret what
came back and return events. The app crate owns the event loop and runs the
blocking work on short-lived worker threads.

That split is the reason 120+ tests run in milliseconds on every platform,
including cases that would otherwise need a day of wall-clock time, a session
expiring, a machine asleep through two warning thresholds, quiet hours
deferring an expiry notice.

### Why one Rust codebase rather than native-per-platform

`tray-icon` wraps `NSStatusItem`, `Shell_NotifyIcon`, and
`libayatana-appindicator` behind one API, and all three accept an RGBA bitmap.
That is what makes the Windows countdown-on-icon trick portable unchanged. The
alternative, three implementations of the estimator, guarantees they drift.

Windows have no such convergence, so the details panel uses the system webview
and inherits the website's CSS. The tray itself stays fully native.

## Decisions worth recording

### Timestamp a session's end at the midpoint

The true expiry lies between the last probe that succeeded and the first that
failed, and cannot be narrowed further. Recording the midpoint halves the
worst-case error; recording the failure would bias every sample late by one full
polling interval. With a ten-minute interval that is the difference between a
5-minute and a 10-minute systematic error, on a measurement whose whole purpose
is precision.

### Tighten the polling near the predicted expiry

Ten minutes normally, two minutes inside the last half hour. The sample that
trains the next estimate is taken exactly when accuracy matters, and the app
spends nothing the rest of the day.

Once the credential is known expired the interval relaxes again, there is
nothing left to measure.

### Require two markers to count a login

The Windows predecessor matched only `Running [gcloud.auth.login]`, which
appears in the log of *every* login attempt including the cancelled ones. An
abandoned login followed two minutes later by a real one therefore looked like a
two-minute session, poisoning the gap heuristic. This implementation requires
the invocation marker *and* a completion marker.

### Timestamp a login by file mtime, not by filename

The macOS predecessor parsed the log filename, which records when the command
*started*. The session begins when the login *completes*, which on a slow
browser hand-off can be minutes later. The file's modification time is the
better proxy.

### The fallback uses the 30th percentile of login gaps, not the median

A gap between logins is an upper bound on the session it followed: you log in
some time *after* expiry, never before. A low percentile removes most of that
bias without modelling it.

### Redraw only when the label changes

On Linux `tray-icon` sets an AppIndicator icon by writing a file and pointing
the indicator at it. An unguarded redraw every five seconds would churn twelve
temp files a minute forever. Gating on the rendered label drops that to at most
one a minute, and to nearly nothing while the countdown reads in hours.

### Text in the icon on Windows and Linux; beside it on macOS

`NSStatusItem` has a title slot, and using it is what every other menu bar app
does. Windows and Linux have no such slot, so the countdown is drawn into the
bitmap.

Only a real countdown earns that space. `!` beside a red dot and `?` beside a
grey one repeat what the colour already said, and every character is taken from
the menu bar's fixed width, permanently, on a notched Mac.

### Quiet hours suppress warnings but consume them; they defer expiry notices

An early warning that arrives six hours late quotes a number that is now wrong,
and the icon has been carrying the same information the whole time, so a
threshold crossed during quiet hours is marked as fired and never shown.

An expiry notice is different: it names no time, so it cannot go stale. It is
suppressed *without* being marked, and fires at the first moment outside quiet
hours.

### Check for updates, never self-update

On every platform something else owns the installed files. A self-updater
writing over them leaves the package manager describing a version that is no
longer on disk.

### Shell out to curl for the update check

One request a day does not justify linking a TLS stack larger than the rest of
the app. `curl` ships with macOS, Windows 10 1803 and later, and effectively
every Linux.

### Migrate the predecessors' state

Session samples cost real wall-clock time, eighteen of them means three weeks
of running the old tray. Discarding that on upgrade would make the new app worse
than the one it replaces on day one. Migration runs once, only when there is no
history of our own, and never overwrites.

The predecessors also share this app's identity (`com.nic.gclouddot`), so their
login items are retired on first launch. Nothing is deleted: the old bundle
stays until the user removes it.

## Things deliberately not done

- **No auto-launch of `gcloud auth login` on expiry.** It would steal focus and
  open a browser unprompted. The notification is one click away from doing it.
- **No Git LFS-style cleverness around service account key age.** The JSON key
  carries no creation date and querying IAM needs a permission the user may not
  have, so the file's modification time is reported as exactly that.
- **No GNOME Shell extension.** Detecting the missing tray and saying so is
  most of the value for none of the maintenance.
- **No sandbox.** The app runs another binary and reads its configuration; a
  sandbox that permitted this would be theatre.

## Testing approach

Every test is a logic test. No windows, no subprocesses, no network, no sleeps.

The cases worth knowing about:

- The real 18-sample dataset from the Windows tray, asserting the estimator
  agrees with Google's published 16 hours.
- A network failure classified as `Indeterminate` rather than expiry.
- A cancelled login rejected by the log scanner.
- A machine asleep through both warning thresholds emitting one notification.
- Quiet hours deferring an expiry notice rather than dropping it.
- A configuration name containing markup, asserting the panel escapes it.
