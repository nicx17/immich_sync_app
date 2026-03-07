import sys
import logging
from log_setup import setup_logging

setup_logging()

from PySide6.QtWidgets import QApplication
from PySide6.QtCore import Qt
from PySide6.QtNetwork import QLocalServer, QLocalSocket
from settings_window import SettingsWindow

def main():
    logging.info("Starting Settings Window (Standalone Process)...")
    
    # In Qt 6 (PySide6), High DPI scaling is enabled by default.
    # No manual attribute setting is required.

    # Set Metadata for DE Integration (GNOME/KDE) MUST be done before QApplication
    # ApplicationName is used for WM_CLASS on some platforms
    QApplication.setApplicationName("immich-sync")
    QApplication.setApplicationDisplayName("Immich Auto-Sync")
    QApplication.setDesktopFileName("immich-sync")

    app = QApplication(sys.argv)
    
    SERVER_NAME = "immich-sync-settings-ipc-v1"
    
    # Try connecting to existing instance
    socket = QLocalSocket()
    socket.connectToServer(SERVER_NAME)
    if socket.waitForConnected(500):
        logging.info("Settings window is already running. Forwarding request and exiting.")
        msg = "about" if "--about" in sys.argv else "show"
        socket.write(msg.encode('utf-8'))
        socket.waitForBytesWritten()
        socket.disconnectFromServer()
        sys.exit(0)

    # We are the single instance, so run the server
    QLocalServer.removeServer(SERVER_NAME)
    server = QLocalServer()
    server.listen(SERVER_NAME)

    window = SettingsWindow()
    window.show()

    def handle_new_connection():
        client = server.nextPendingConnection()
        if client:
            client.waitForReadyRead(500)
            msg = bytes(client.readAll().data()).decode('utf-8')
            
            # Bring window to front
            window.setWindowState(window.windowState() & ~Qt.WindowState.WindowMinimized | Qt.WindowState.WindowActive)
            window.raise_()
            window.activateWindow()
            
            if msg == "about":
                window.show_about_dialog()
            
            client.disconnectFromServer()

    server.newConnection.connect(handle_new_connection)

    if "--about" in sys.argv:
        window.show_about_dialog()
    
    sys.exit(app.exec())

if __name__ == "__main__":
    main()