import sys
import os
import logging
from PySide6.QtWidgets import (QApplication, QWidget, QVBoxLayout, QHBoxLayout, 
                               QLabel, QLineEdit, QPushButton, QListWidget, 
                               QListWidgetItem, QMessageBox, QFileDialog, QFormLayout, QProgressBar)
from PySide6.QtGui import QIcon
from PySide6.QtCore import Qt, QTimer
from config import Config
from api_client import ImmichApiClient
from state_manager import StateManager

class SettingsWindow(QWidget):
    def __init__(self, config_manager=None, monitor=None):
        super().__init__()
        self.setWindowTitle("Immich Auto-Sync Settings")
        
        # Set Window Icon
        # Prioritize local asset file for reliability
        icon_path = os.path.join(os.path.dirname(__file__), "assets", "icon.png")
        if os.path.exists(icon_path):
            self.setWindowIcon(QIcon(icon_path))
        elif QIcon.hasThemeIcon("immich-sync"):
            self.setWindowIcon(QIcon.fromTheme("immich-sync"))
            
        self.resize(600, 500)
        self._set_modern_style()
        
        # When running standalone, config_manager might be None
        self.config = config_manager if config_manager else Config()
        self.monitor = monitor # Might be None if running standalone
        
        self._init_ui()
        self._load_values()

        self.state_manager = StateManager()
        self.timer = QTimer(self)
        self.timer.timeout.connect(self.update_progress)
        self.timer.start(500)

    def update_progress(self):
        state = self.state_manager.read_state()
        if not state:
            return
            
        status = state.get('status', 'idle')
        progress = state.get('progress', 0)
        processed = state.get('processed_count', 0)
        total = state.get('total_queued', 0)
        current_file = state.get('current_file')
        
        if status == 'idle':
            self.status_label.setText(f"Status: Idle (Processed {processed} files)")
            # If we processed something, keep bar full, else 0.
            self.progress_bar.setValue(100 if processed > 0 else 0)
        elif status == 'uploading':
            filename = os.path.basename(current_file) if current_file else "..."
            self.status_label.setText(f"Uploading ({processed}/{total}): {filename}")
            self.progress_bar.setValue(progress)

    def _set_modern_style(self):
        # Apply a sleek dark theme
        self.setStyleSheet("""
            QWidget {
                background-color: #1e1e1e;
                color: #e0e0e0;
                font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
                font-size: 18px;
                border: none;
            }
            QLineEdit {
                background-color: #2d2d2d;
                border: 1px solid #444;
                border-radius: 6px;
                padding: 10px;
                color: #ffffff;
                selection-background-color: #4285f4;
            }
            QLineEdit:hover {
                border: 1px solid #666;
            }
            QLineEdit:focus {
                border: 1px solid #4285f4;
                background-color: #333;
            }
            QPushButton {
                background-color: #3c3c3c;
                border: 1px solid #555;
                color: #eee;
                border-radius: 6px;
                padding: 10px 20px;
                font-weight: 600;
                min-width: 100px;
            }
            QPushButton:hover {
                background-color: #4c4c4c;
                border: 1px solid #777;
            }
            QPushButton:pressed {
                background-color: #4285f4; 
                border-color: #4285f4;
                color: #fff;
            }
            QListWidget {
                background-color: #2d2d2d;
                border: 1px solid #444;
                border-radius: 6px;
                padding: 5px;
            }
            QListWidget::item {
                padding: 8px 12px;
                border-radius: 4px;
                color: #eee;
            }
            QListWidget::item:selected {
                background-color: #4285f4;
                color: white;
            }
            QListWidget::item:hover:!selected {
                background-color: #383838;
            }
            QLabel {
                color: #ccc;
                font-weight: 500;
                margin-top: 5px;
            }
            QProgressBar {
                border: none;
                background-color: #333333;
                border-radius: 10px;
                text-align: center;
                color: #ffffff;
                font-weight: bold;
                font-size: 13px;
            }
            QProgressBar::chunk {
                background-color: qlineargradient(spread:pad, x1:0, y1:0, x2:1, y2:0, stop:0 #4285f4, stop:1 #6ea1f7);
                border-radius: 10px;
            }
            QMessageBox {
                background-color: #1e1e1e;
                color: #fff;
            }
            QMessageBox QLabel {
                color: #fff;
            }
            QMessageBox QPushButton {
                min-width: 80px;
            }
        """)

    def _init_ui(self):
        layout = QVBoxLayout()
        layout.setContentsMargins(30, 30, 30, 30)
        layout.setSpacing(15)
        
        # --- Connection Header ---
        conn_header = QLabel("Connectivity")
        conn_header.setStyleSheet("font-size: 18px; font-weight: bold; color: #4285f4; margin-bottom: 5px;")
        layout.addWidget(conn_header)

        # --- Connection Form ---
        conn_layout = QFormLayout()
        conn_layout.setSpacing(15)
        conn_layout.setLabelAlignment(Qt.AlignmentFlag.AlignRight)
        
        self.internal_url_input = QLineEdit()
        self.internal_url_input.setPlaceholderText("http://192.168.1.10:2283")
        conn_layout.addRow("Internal URL (LAN):", self.internal_url_input)
        
        self.external_url_input = QLineEdit()
        self.external_url_input.setPlaceholderText("https://immich.example.com")
        conn_layout.addRow("External URL (WAN):", self.external_url_input)
        
        self.api_key_input = QLineEdit()
        self.api_key_input.setEchoMode(QLineEdit.EchoMode.Password)
        self.api_key_input.setPlaceholderText("Paste API Key here")
        conn_layout.addRow("API Key:", self.api_key_input)
        
        layout.addLayout(conn_layout)
        
        # Test Connection Button
        self.test_btn = QPushButton("Test Connection")
        self.test_btn.clicked.connect(self.test_connection)
        layout.addWidget(self.test_btn)
        
        layout.addSpacing(20)
        
        # --- Progress ---
        layout.addSpacing(10)
        status_header = QLabel("Sync Status")
        status_header.setStyleSheet("font-size: 18px; font-weight: bold; color: #4285f4; margin-top: 5px;")
        layout.addWidget(status_header)

        self.status_label = QLabel("Status: Idle")
        layout.addWidget(self.status_label)
        
        self.progress_bar = QProgressBar()
        self.progress_bar.setRange(0, 100)
        self.progress_bar.setValue(0)
        self.progress_bar.setFixedHeight(20) # Make it slimmer
        layout.addWidget(self.progress_bar)
        
        layout.addSpacing(10)

        # --- Watch Paths ---
        layout.addSpacing(10)
        watch_header = QLabel("Watch Folders")
        watch_header.setStyleSheet("font-size: 18px; font-weight: bold; color: #4285f4; margin-top: 10px; margin-bottom: 5px;")
        layout.addWidget(watch_header)
        
        self.path_list = QListWidget()
        layout.addWidget(self.path_list)
        
        path_btn_layout = QHBoxLayout()
        self.add_path_btn = QPushButton("+ Add Folder")
        self.add_path_btn.clicked.connect(self.add_path)
        path_btn_layout.addWidget(self.add_path_btn)
        
        self.remove_path_btn = QPushButton("- Remove Folder")
        self.remove_path_btn.clicked.connect(self.remove_path)
        path_btn_layout.addWidget(self.remove_path_btn)
        
        layout.addLayout(path_btn_layout)
        
        layout.addStretch()
        
        # --- Bottom Buttons ---
        bottom_layout = QHBoxLayout()
        
        self.close_btn = QPushButton("Close")
        self.close_btn.clicked.connect(self.close)
        bottom_layout.addWidget(self.close_btn)
        
        self.save_btn = QPushButton("Save && Restart")
        self.save_btn.clicked.connect(self.save_settings)
        bottom_layout.addWidget(self.save_btn)
        
        layout.addLayout(bottom_layout)
        
        self.setLayout(layout)

    def _load_values(self):
        self.internal_url_input.setText(self.config.internal_url)
        self.external_url_input.setText(self.config.external_url)
        
        api_key = self.config.get_api_key()
        if api_key:
            self.api_key_input.setText(api_key)
            
        self.path_list.clear()
        for path in self.config.watch_paths:
            self.path_list.addItem(path)

    def add_path(self):
        folder = QFileDialog.getExistingDirectory(self, "Select Folder to Watch")
        if folder:
            # Check if already exists
            items = [self.path_list.item(i).text() for i in range(self.path_list.count())]
            if folder not in items:
                self.path_list.addItem(folder)

    def remove_path(self):
        current_row = self.path_list.currentRow()
        if current_row >= 0:
            self.path_list.takeItem(current_row)

    def test_connection(self):
        internal = self.internal_url_input.text().strip()
        external = self.external_url_input.text().strip()
        api_key = self.api_key_input.text().strip()
        
        logging.info(f"Testing connectivity to Internal: {internal}, External: {external}")
        
        # Use transient client for testing
        client = ImmichApiClient(internal, external, api_key)
        
        # Test Internal explicitly
        internal_status = "N/A"
        internal_ok = False
        if internal:
            logging.info(f"Testing Internal URL: {client.internal_url}")
            internal_ok = client._ping(client.internal_url)
            internal_status = "OK" if internal_ok else "FAILED"
            
        # Test External explicitly
        external_status = "N/A"
        external_ok = False
        if external:
            logging.info(f"Testing External URL: {client.external_url}")
            external_ok = client._ping(client.external_url)
            external_status = "OK" if external_ok else "FAILED"

        # Construct Report
        report = (
            f"Internal Connection: {internal_status}\n"
            f"External Connection: {external_status}\n"
        )
        
        if internal_ok:
            report += f"\nActive Mode: LAN ({client.internal_url})"
            QMessageBox.information(self, "Connection Test - Success", report)
        elif external_ok:
            report += f"\nActive Mode: WAN ({client.external_url})"
            QMessageBox.information(self, "Connection Test - Success", report)
        else:
            report += "\nCould not connect to Immich at either address."
            QMessageBox.critical(self, "Connection Test - Failed", report)

    def save_settings(self):
        logging.info("Saving settings...")
        # Update config object locally
        self.config.data["internal_url"] = self.internal_url_input.text().strip()
        self.config.data["external_url"] = self.external_url_input.text().strip()
        
        # Collect paths
        paths = []
        for i in range(self.path_list.count()):
            paths.append(self.path_list.item(i).text())
        self.config.data["watch_paths"] = paths
        
        logging.info(f"Saving {len(paths)} watch paths: {paths}")
        
        # Save JSON
        self.config.save()
        logging.info("Configuration saved to disk.")
        
        # Save Keyring
        key = self.api_key_input.text().strip()
        if key:
            self.config.set_api_key(key)
            logging.info("API Key updated in keyring.")
            
        QMessageBox.information(self, "Saved", "Settings saved. Please restart the app for changes to take effect fully.")
        self.close()

if __name__ == "__main__":
    app = QApplication(sys.argv)
    window = SettingsWindow()
    window.show()
    sys.exit(app.exec())
