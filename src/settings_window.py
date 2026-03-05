import sys
import os
import logging
from PySide6.QtWidgets import (
    QTableWidget, QTableWidgetItem, QHeaderView, QComboBox,QApplication, QWidget, QVBoxLayout, QHBoxLayout, 
                               QLabel, QLineEdit, QPushButton, QListWidget, 
                               QListWidgetItem, QMessageBox, QFileDialog, QFormLayout, QProgressBar, QTextEdit, QDialog)
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
                background-color: #000000;
                color: #ffffff;
                font-family: "Inter", "Roboto", "Ubuntu", "Segoe UI", system-ui, sans-serif;
                font-size: 18px;
                border: none;
            }
            QLineEdit {
                background-color: #111111;
                border: 2px solid #444;
                border-radius: 6px;
                padding: 10px;
                color: #ffffff;
                selection-background-color: #ffffff;
                selection-color: #000000;
            }
            QLineEdit:hover {
                border: 2px solid #666;
            }
            QLineEdit:focus {
                border: 2px solid #ffffff;
                background-color: #222222;
            }
            QPushButton {
                background-color: #1a1a1a;
                border: 2px solid #555;
                color: #ffffff;
                border-radius: 6px;
                padding: 10px 20px;
                font-weight: 600;
                min-width: 100px;
            }
            QPushButton:hover {
                background-color: #333333;
                border: 2px solid #888;
            }
            QPushButton:pressed {
                background-color: #ffffff; 
                border-color: #ffffff;
                color: #000000;
            }
            QListWidget {
                background-color: #111111;
                border: 2px solid #444;
                border-radius: 6px;
                padding: 5px;
            }
            QListWidget::item {
                padding: 8px 12px;
                border-radius: 4px;
                color: #ffffff;
            }
            QListWidget::item:selected {
                background-color: #ffffff;
                color: #000000;
            }
            QListWidget::item:hover:!selected {
                background-color: #222222;
            }
            QLabel {
                color: #dddddd;
                font-weight: 500;
                margin-top: 5px;
            }
            QProgressBar {
                border: none;
                background-color: #222222;
                border-radius: 10px;
                text-align: center;
                color: #000000;
                font-weight: bold;
                font-size: 14px;
            }
            QProgressBar::chunk {
                background-color: #ffffff;
                border-radius: 10px;
            }
            QMessageBox {
                background-color: #000000;
                color: #ffffff;
            }
            QMessageBox QLabel {
                color: #ffffff;
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
        conn_header.setStyleSheet("font-size: 18px; font-weight: bold; color: #ffffff; margin-bottom: 5px;")
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
        status_header.setStyleSheet("font-size: 18px; font-weight: bold; color: #ffffff; margin-top: 5px;")
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
        watch_header.setStyleSheet("font-size: 18px; font-weight: bold; color: #ffffff; margin-top: 10px; margin-bottom: 5px;")
        layout.addWidget(watch_header)
        
        self.path_list = QTableWidget(0, 2)
        self.path_list.setHorizontalHeaderLabels(["Folder Path", "Target Immich Album"])
        self.path_list.horizontalHeader().setSectionResizeMode(0, QHeaderView.Stretch)
        self.path_list.horizontalHeader().setSectionResizeMode(1, QHeaderView.ResizeToContents)
        self.path_list.setSelectionBehavior(QTableWidget.SelectRows)
        self.path_list.setEditTriggers(QTableWidget.NoEditTriggers)
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
        
        self.about_btn = QPushButton("About")
        self.about_btn.clicked.connect(self.show_about_dialog)
        bottom_layout.addWidget(self.about_btn)

        bottom_layout.addStretch()

        self.close_btn = QPushButton("Close")
        self.close_btn.clicked.connect(self.close)
        bottom_layout.addWidget(self.close_btn)
        
        self.save_btn = QPushButton("Save && Restart")
        self.save_btn.clicked.connect(self.save_settings)
        bottom_layout.addWidget(self.save_btn)
        
        layout.addLayout(bottom_layout)
        
        self.setLayout(layout)

    def show_about_dialog(self):
        QMessageBox.about(self, "About Immich Auto-Sync", 
            "<h3>Immich Auto-Sync</h3>"
            "<p>A daemon-based synchronization tool for automatically uploading media files to an Immich server.</p>"
            "<p>Version: 0.2.0-beta<br/>"
            "License: GPLv3</p>"
            "<p>Icon by Round Icons on Unsplash.</p>"
            "<p><a href='https://github.com/nicx17/immich_sync_app'>https://github.com/nicx17/immich_sync_app</a></p>")


    def _fetch_remote_albums(self):
        internal = self.config.internal_url
        external = self.config.external_url
        api_key = self.config.get_api_key()
        if api_key and (internal or external):
            client = ImmichApiClient(internal, external, api_key)
            # Try to fetch without failing hard
            try:
                if client.check_connection():
                    return client.get_albums()
            except Exception:
                pass
        return []

    def _load_values(self):
        self.internal_url_input.setText(self.config.internal_url)
        self.external_url_input.setText(self.config.external_url)
        
        api_key = self.config.get_api_key()
        if api_key:
            self.api_key_input.setText(api_key)
            
        # Try to fetch albums for the dropdown
        self.remote_albums = self._fetch_remote_albums()
            
        self.path_list.setRowCount(0)
        for p in self.config.watch_paths:
            if isinstance(p, dict):
                self._add_path_to_table(p["path"], p.get("album_id"), p.get("album_name"))
            else:
                self._add_path_to_table(p, None, None)

    def _add_path_to_table(self, folder, current_album_id=None, current_album_name=None):
        row = self.path_list.rowCount()
        self.path_list.insertRow(row)
        
        path_item = QTableWidgetItem(folder)
        self.path_list.setItem(row, 0, path_item)
        
        combo = QComboBox()
        combo.setEditable(True)
        combo.setInsertPolicy(QComboBox.NoInsert)
        combo.setToolTip("Select an existing album, or type a new album name to create.")
        
        combo.addItem("Default (Folder Name)", userData=None)
        
        # Populate with remote albums
        if hasattr(self, 'remote_albums') and self.remote_albums:
            for album in self.remote_albums:
                combo.addItem(album['albumName'], userData=album['id'])
                
        # Set selection if it exists
        if current_album_id:
            idx = combo.findData(current_album_id)
            if idx >= 0:
                combo.setCurrentIndex(idx)
        elif current_album_name and current_album_name != "Default (Folder Name)":
            # If there's no ID, but a custom name was typed, show it
            combo.setCurrentText(current_album_name)
        
        self.path_list.setCellWidget(row, 1, combo)

    def add_path(self):
        folder = QFileDialog.getExistingDirectory(self, "Select Folder to Watch")
        if folder:
            # Check if already exists
            items = []
            for i in range(self.path_list.rowCount()):
                item = self.path_list.item(i, 0)
                if item:
                    items.append(item.text())
                    
            if folder not in items:
                self._add_path_to_table(folder)

    def remove_path(self):
        current_row = self.path_list.currentRow()
        if current_row >= 0:
            self.path_list.removeRow(current_row)

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
        for i in range(self.path_list.rowCount()):
            path_item = self.path_list.item(i, 0)
            if not path_item: continue
            folder = path_item.text()
            
            combo = self.path_list.cellWidget(i, 1)
            if combo:
                album_name = combo.currentText().strip()
                # Check if this exact text exists in the list to determine if it's custom
                idx = combo.findText(album_name)
                if idx >= 0:
                    album_id = combo.itemData(idx)
                else:
                    album_id = None # Custom text implies a new uncreated album
                
                if not album_name:
                    album_name = "Default (Folder Name)"
            else:
                album_name = "Default (Folder Name)"
                album_id = None
            
            paths.append({
                "path": folder,
                "album_id": album_id,
                "album_name": album_name
            })
            
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
