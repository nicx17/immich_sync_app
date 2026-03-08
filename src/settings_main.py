import sys
import os
import logging
logger = logging.getLogger(__name__)
from log_setup import setup_logging

setup_logging()

import gi
gi.require_version('Gtk', '4.0')
gi.require_version('Adw', '1')
from gi.repository import Gtk, Adw, GLib, Gio
from settings_window import SettingsWindow

class ImmichSyncApp(Adw.Application):
    def __init__(self):
        super().__init__(
            application_id="com.github.nicx17.mimick",
            flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE
        )
        self.settings_window = None

    def do_activate(self):
        logger.info("Settings App activated.")
        if not self.settings_window:
            self.settings_window = SettingsWindow(application=self)
        self.settings_window.present()

    def do_command_line(self, command_line):
        self.activate()
        args = command_line.get_arguments()
        if "--about" in args:
            if self.settings_window:
                self.settings_window.show_about_dialog()
        return 0

def main():
    logger.info("Starting Settings GTK4 Window (Standalone Process)...")
    app = ImmichSyncApp()
    sys.exit(app.run(sys.argv))

if __name__ == "__main__":
    main()
