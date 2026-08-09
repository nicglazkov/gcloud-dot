#!/usr/bin/env python3
"""Build the installer disk image.

    build-dmg.py <app bundle> <output.dmg> <background base>

The background base is the path prefix produced by the render_dmg_background
example, without an extension. This script combines its 1x and 2x renders into
the multi-resolution TIFF that Finder needs to stay sharp on a Retina display.

Why dmgbuild rather than create-dmg: create-dmg drives Finder through
AppleScript to place the icons, which needs a GUI session and an automation
grant. dmgbuild writes the .DS_Store directly, so the layout is produced by
code rather than by remote-controlling a window that may not exist.
"""

import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    app, out, bg_base = sys.argv[1], sys.argv[2], sys.argv[3]

    if not os.path.isdir(app):
        print(f"no app bundle at {app}", file=sys.stderr)
        return 1

    try:
        import dmgbuild
        import dmgbuild.core as core
    except ImportError:
        print("dmgbuild is required: python3 -m pip install --user dmgbuild",
              file=sys.stderr)
        return 1

    # Combine the two renders. Finder picks the representation matching the
    # display, so a Retina screen gets the 2x one instead of an upscaled 1x.
    background = os.path.join(tempfile.mkdtemp(prefix="dmgbg-"), "background.tiff")
    one, two = f"{bg_base}.png", f"{bg_base}@2x.png"
    if os.path.exists(one) and os.path.exists(two):
        subprocess.check_call(
            ["tiffutil", "-cathidpicheck", one, two, "-out", background],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    elif os.path.exists(one):
        background = one
    else:
        print(f"no background at {one}", file=sys.stderr)
        return 1

    # dmgbuild attaches without -mountpoint, so hdiutil picks a name under
    # /Volumes. Environments that forbid creating mount points there fail with
    # a bare "Operation not permitted" naming a path inside the volume, which
    # points at everything except the cause. Every hdiutil call goes through
    # one helper, so redirecting the mount is a two line shim.
    real_hdiutil = core.hdiutil
    mountpoint = tempfile.mkdtemp(prefix="dmgmnt-")

    def hdiutil(cmd, *args, **kwargs):
        if cmd == "attach":
            args = tuple(args) + ("-mountpoint", mountpoint)
        return real_hdiutil(cmd, *args, **kwargs)

    core.hdiutil = hdiutil

    if os.path.exists(out):
        os.remove(out)

    os.environ["GCLOUD_DOT_APP"] = os.path.abspath(app)
    os.environ["GCLOUD_DOT_BACKGROUND"] = background
    dmgbuild.build_dmg(
        out, "GCloud Dot",
        settings_file=os.path.join(HERE, "dmg-settings.py"),
    )
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
