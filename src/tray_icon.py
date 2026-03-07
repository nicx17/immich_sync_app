import logging
import threading
import subprocess
import sys
import os
from PIL import Image, ImageDraw
import pystray
from pystray import MenuItem as item

# NOTE: Removed PySide6 from here to avoid conflicts.
# Settings Window is now a separate process.

class TrayIcon:
    def __init__(self, monitor):
        logging.info("Initializing TrayIcon...")
        self.monitor = monitor
        self.icon = None
        self._exiting_cleanly = False
        
        # Create the icon
        logging.info("Creating icon image...")
        image = self._create_image()
        
        logging.info("Creating pystray Icon instance...")
        # Try simplified icon first to debug GTK Critical error
        self.icon = pystray.Icon(
            "immich-sync", 
            image, 
            "Immich Auto-Sync", 
            menu=pystray.Menu(
                item('Settings', self.show_settings),
                item('About', self.show_about),
                item('Quit', self.quit_app)
            )
        )
        logging.info("TrayIcon initialized.")

    def _create_image(self):
        # Load icon from assets if available
        icon_path = os.path.join(os.path.dirname(__file__), "assets", "icon.png")
        if os.path.exists(icon_path):
            try:
                logging.info(f"Loading icon from {icon_path}")
                return Image.open(icon_path)
            except Exception as e:
                logging.error(f"Failed to load icon from file: {e}")
        
        # Fallback: Generate a default icon
        logging.warning("Icon file not found or failed to load. Using fallback icon.")
        width = 64
        height = 64
        color1 = (66, 133, 244)
        color2 = (255, 255, 255)

        image = Image.new('RGBA', (width, height), color1)
        dc = ImageDraw.Draw(image)
        dc.rectangle((28, 20, 36, 44), fill=color2)
        
        return image

    def show_settings(self, icon, item):
        logging.info("Opening Settings Window...")
        # Use subprocess to launch the settings window as a separate process
        # This completely avoids the Qt vs GTK loop conflict
        script_path = os.path.join(os.path.dirname(__file__), "settings_main.py")
        subprocess.Popen([sys.executable, script_path])


    def show_about(self, icon, item):
        logging.info("Opening About Dialog...")
        script_path = os.path.join(os.path.dirname(__file__), "settings_main.py")
        subprocess.Popen([sys.executable, script_path, "--about"])

    def run(self):
        # This blocks in pystray
        logging.info("Starting System Tray Icon (Main Loop)...")
        if self.icon:
             logging.info("Calling self.icon.run()")
             try:
                self.icon.run()
             except Exception as e:
                logging.error(f"Error in tray icon run loop: {e}", exc_info=True)
                raise RuntimeError(f"Tray icon failed to run: {e}")
        else:
            logging.error("Icon instance is None!")
            raise RuntimeError("Tray icon instance is None, failed to initialize.")

    def stop(self):
        logging.info("Stopping System Tray Icon...")
        if self.icon:
            self.icon.stop()
        if self.monitor:
            self.monitor.stop()

    def quit_app(self, icon, item):
        self._exiting_cleanly = True
        self.stop()
        # Clean exit
        sys.exit(0)
