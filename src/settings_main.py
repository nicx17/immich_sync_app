import sys
import logging
from PySide6.QtWidgets import QApplication
from PySide6.QtCore import Qt
from settings_window import SettingsWindow

# Configure logging
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')

def main():
    logging.info("Starting Settings Window (Standalone Process)...")
    
    # In Qt 6 (PySide6), High DPI scaling is enabled by default.
    # No manual attribute setting is required.

    app = QApplication(sys.argv)
    
    # Set Metadata for DE Integration (GNOME/KDE)
    # ApplicationName is used for WM_CLASS on some platforms
    app.setApplicationName("immich-sync")
    app.setApplicationDisplayName("Immich Auto-Sync")
    app.setDesktopFileName("immich-sync.desktop")
    
    # Run the window
    window = SettingsWindow()
    window.show()
    
    sys.exit(app.exec())

if __name__ == "__main__":
    main()