# dmgbuild settings for the macOS .dmg (scripts/package-macos.sh):
#   dmgbuild -s dmg-settings.py -D app=<Scratchpad.app> -D background=<tiff> Scratchpad <out.dmg>
# The icon positions match the empty frames in dmg-background.svg (660x440 points; the strip
# is the top 420, the rest is plain paper).
# `defines` is provided by dmgbuild.
import os

app = defines["app"]  # noqa: F821
app_name = os.path.basename(app)

format = "UDZO"
files = [app]
symlinks = {"Applications": "/Applications"}

background = defines["background"]  # noqa: F821
# Finder's window bounds include the title bar: 32 pt on current macOS, 28 pt on 11-15, 22 pt before.
# 420 + 32 shows the whole strip everywhere, and the 440 pt background still fills the window
# where the title bar is shorter.
window_rect = ((200, 140), (660, 452))
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
