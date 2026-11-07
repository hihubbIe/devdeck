#!/usr/bin/env python3
"""Takes a full-screen screenshot through the xdg-desktop-portal and copies it to the
path given as the only argument. Used for visual checks during development
(docs/WORKFLOW.md step 5). Works on Wayland and X11 desktops that run the portal."""

import shutil
import sys
from urllib.parse import unquote, urlparse

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

out = sys.argv[1]
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
loop = GLib.MainLoop()
result = {}


def on_response(_conn, _sender, _path, _iface, _signal, params):
    code, results = params.unpack()
    result["code"] = code
    result["uri"] = results.get("uri")
    loop.quit()


token = "devdeck_shot"
sender = bus.get_unique_name()[1:].replace(".", "_")
handle = f"/org/freedesktop/portal/desktop/request/{sender}/{token}"
bus.signal_subscribe(
    "org.freedesktop.portal.Desktop",
    "org.freedesktop.portal.Request",
    "Response",
    handle,
    None,
    Gio.DBusSignalFlags.NO_MATCH_RULE,
    on_response,
)
bus.call_sync(
    "org.freedesktop.portal.Desktop",
    "/org/freedesktop/portal/desktop",
    "org.freedesktop.portal.Screenshot",
    "Screenshot",
    GLib.Variant("(sa{sv})", ("", {"interactive": GLib.Variant("b", False), "handle_token": GLib.Variant("s", token)})),
    None,
    Gio.DBusCallFlags.NONE,
    -1,
    None,
)
GLib.timeout_add_seconds(60, loop.quit)
loop.run()
if result.get("code") != 0 or not result.get("uri"):
    sys.exit(f"screenshot failed: {result}")
shutil.move(unquote(urlparse(result["uri"]).path), out)
print(out)
