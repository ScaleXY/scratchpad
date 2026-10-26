# dmgbuild settings for the macOS .dmg (scripts/package-macos.sh):
#   dmgbuild -s dmg-settings.py -D app=<Scratchpad.app> -D background=<tiff> Scratchpad <out.dmg>
# The icon positions match the empty frames in dmg-background.svg (660x420 points).
# `defines` is provided by dmgbuild.
import os

app = defines["app"]  # noqa: F821
app_name = os.path.basename(app)

format = "UDZO"
files = [app]
symlinks = {"Applications": "/Applications"}

background = defines["background"]  # noqa: F821
window_rect = ((200, 140), (660, 420))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False

icon_size = 112
text_size = 13
icon_locations = {
    app_name: (170, 302),
    "Applications": (490, 302),
}
