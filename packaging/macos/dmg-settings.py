# Layout of the installer window. Read by dmgbuild, driven by build-dmg.py.
#
# The coordinates are in the same point space as the background image, so the
# arrow painted there lands between the two icons rather than near them.

import os

app_path = os.environ["GCLOUD_DOT_APP"]
app_name = os.path.basename(app_path)

files = [app_path]
symlinks = {"Applications": "/Applications"}

format = "UDZO"
compression_level = 9

background = os.environ["GCLOUD_DOT_BACKGROUND"]

# 660 by 420 of content, positioned far enough down that the window does not
# open under the menu bar on a laptop display.
window_rect = ((160, 160), (660, 420))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False

icon_size = 120
text_size = 13
icon_locations = {
    app_name: (170, 200),
    "Applications": (490, 200),
}

# Nothing else belongs in the window. Without this, Finder happily shows the
# trash and fsevents bookkeeping that every disk image carries.
hide_extension = [app_name]
