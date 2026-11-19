# SPDX-License-Identifier: GPL-3.0-or-later
# dmgbuild settings for the battbar disk image: a Finder window with battbar.app on the
# left, an Applications link on the right and an arrow between them (background).
# Built by scripts/build-dmg.sh; values come in through -D.
import os.path

application = defines["app"]  # noqa: F821 (provided by dmgbuild)
appname = os.path.basename(application)

format = "UDZO"
filesystem = "HFS+"
files = [application]
symlinks = {"Applications": "/Applications"}
icon = defines.get("volume_icon")  # noqa: F821
background = defines["background"]  # noqa: F821

# Window: content area 660 × 420 (the background size); Finder adds the title bar.
window_rect = ((240, 160), (660, 448))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False
include_icon_view_settings = True
arrange_by = None
icon_size = 112
text_size = 13
icon_locations = {
    appname: (165, 205),
    "Applications": (495, 205),
}
