import sys
import os
import logging
logger = logging.getLogger(__name__)
import gi

gi.require_version('Gtk', '4.0')
gi.require_version('Adw', '1')
from gi.repository import Gtk, Adw, GLib, Gio

from config import Config
from api_client import ImmichApiClient
from state_manager import StateManager

class SettingsWindow(Adw.ApplicationWindow):
    def __init__(self, application, monitor=None):
        super().__init__(application=application)
        self.set_title("Mimick Settings")
        self.set_default_size(600, 900)
        
        # Enforce Dark Theme
        Adw.StyleManager.get_default().set_color_scheme(Adw.ColorScheme.FORCE_DARK)
        
        # When running standalone, config might be initialized here
        self.config = Config()
        self.monitor = monitor
        self.state_manager = StateManager()

        self.remote_albums = []
        
        self._build_ui()
        self._load_values()

        # Update progress bar safely from background
        GLib.timeout_add(500, self.update_progress)

    def _build_ui(self):
        # We use a Box container holding a HeaderBar and a Scrollable Window for settings
        vbox = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.set_content(vbox)

        # HeaderBar
        header = Adw.HeaderBar()
        vbox.append(header)

        # About Button
        about_btn = Gtk.Button(icon_name="help-about-symbolic")
        about_btn.set_tooltip_text("About Mimick")
        about_btn.connect("clicked", self.on_about_clicked)
        header.pack_start(about_btn)

        # Main scrollable area
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_vexpand(True)
        vbox.append(scroll)

        # Clamp to keep settings centered/max width but let height expand
        clamp = Adw.Clamp()
        clamp.set_maximum_size(600)
        clamp.set_margin_top(12)
        clamp.set_margin_bottom(12)
        clamp.set_margin_start(12)
        clamp.set_margin_end(12)
        scroll.set_child(clamp)

        # Main Page Box
        page_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=24)
        clamp.set_child(page_box)

        # --- PROGRESS GROUP ---
        progress_group = Adw.PreferencesGroup(title="Sync Status")
        page_box.append(progress_group)

        self.status_row = Adw.ActionRow(title="Idle")
        self.status_row.set_subtitle("Waiting to sync...")
        progress_group.add(self.status_row)

        self.progress_bar = Gtk.ProgressBar()
        self.progress_bar.set_margin_top(12)
        self.progress_bar.set_margin_bottom(12)
        self.progress_bar.set_margin_start(12)
        self.progress_bar.set_margin_end(12)
        self.progress_bar.set_fraction(0.0)
        progress_group.add(self.progress_bar)

        # --- CONNECTIVITY GROUP ---
        conn_group = Adw.PreferencesGroup(title="Connectivity")
        page_box.append(conn_group)

        # Internal URL
        self.internal_row = Adw.ActionRow(title="Internal URL (LAN)")
        self.internal_switch = Gtk.Switch()
        self.internal_switch.set_valign(Gtk.Align.CENTER)
        self.internal_switch.connect("notify::active", self._validate_toggles)
        self.internal_row.add_prefix(self.internal_switch)
        
        self.internal_entry = Gtk.Entry(placeholder_text="http://192.168.1.10:2283")
        self.internal_entry.set_valign(Gtk.Align.CENTER)
        self.internal_entry.set_hexpand(True)
        self.internal_row.add_suffix(self.internal_entry)
        conn_group.add(self.internal_row)

        # External URL
        self.external_row = Adw.ActionRow(title="External URL (WAN)")
        self.external_switch = Gtk.Switch()
        self.external_switch.set_valign(Gtk.Align.CENTER)
        self.external_switch.connect("notify::active", self._validate_toggles)
        self.external_row.add_prefix(self.external_switch)

        self.external_entry = Gtk.Entry(placeholder_text="https://immich.example.com")
        self.external_entry.set_valign(Gtk.Align.CENTER)
        self.external_entry.set_hexpand(True)
        self.external_row.add_suffix(self.external_entry)
        conn_group.add(self.external_row)

        # API Key
        self.api_key_row = Adw.ActionRow(title="API Key")
        self.api_key_entry = Gtk.PasswordEntry()
        self.api_key_entry.set_valign(Gtk.Align.CENTER)
        self.api_key_entry.set_hexpand(True)
        self.api_key_row.add_suffix(self.api_key_entry)
        conn_group.add(self.api_key_row)

        # Test Connection Button
        test_btn = Gtk.Button(label="Test Connection")
        test_btn.set_margin_top(12)
        test_btn.connect("clicked", self.on_test_connection_clicked)
        conn_group.add(test_btn)


        # --- WATCH FOLDERS GROUP ---
        self.folders_group = Adw.PreferencesGroup(title="Watch Folders")
        page_box.append(self.folders_group)
        
        # We list existing folders as ActionRows dynamically
        self.add_folder_btn = Gtk.Button(label="Add Folder")
        self.add_folder_btn.set_margin_top(12)
        self.add_folder_btn.connect("clicked", self.on_add_folder_clicked)
        self._tracked_group_widgets = []
        save_group = Adw.PreferencesGroup()
        page_box.append(save_group)
        
        save_btn = Gtk.Button(label="Save & Restart")
        save_btn.add_css_class("suggested-action")
        save_btn.connect("clicked", self.on_save_clicked)
        save_group.add(save_btn)


    def update_progress(self):
        state = self.state_manager.read_state()
        if not state:
            return True # Continue timer
            
        status = state.get('status', 'idle')
        progress = state.get('progress', 0)
        processed = state.get('processed_count', 0)
        total = state.get('total_queued', 0)
        current_file = state.get('current_file')
        
        if status == 'idle':
            self.status_row.set_title(f"Idle")
            self.status_row.set_subtitle(f"Processed {processed} files")
            self.progress_bar.set_fraction(1.0 if processed > 0 else 0.0)
        elif status == 'uploading':
            filename = os.path.basename(current_file) if current_file else "..."
            self.status_row.set_title(f"Uploading ({processed}/{total})")
            self.status_row.set_subtitle(filename)
            self.progress_bar.set_fraction(progress / 100.0)
            
        return True # Continue GLib timer

    def _fetch_remote_albums(self):
        internal = self.config.internal_url if self.config.internal_url_enabled else ""
        external = self.config.external_url if self.config.external_url_enabled else ""
        api_key = self.config.get_api_key()
        if api_key and (internal or external):
            client = ImmichApiClient(internal, external, api_key)
            try:
                if client.check_connection():
                    return client.get_albums()
            except Exception:
                pass
        return []

    def _validate_toggles(self, switch, gparam):
        if not self.internal_switch.get_active() and not self.external_switch.get_active():
            # Revert the one that was just disabled
            switch.set_active(True)
            
            # Show a dialog
            dialog = Adw.MessageDialog(
                transient_for=self,
                heading="Invalid Selection",
                body="At least one URL (Internal or External) must be enabled."
            )
            dialog.add_response("ok", "OK")
            dialog.present()
            
        self.internal_entry.set_sensitive(self.internal_switch.get_active())
        self.external_entry.set_sensitive(self.external_switch.get_active())

    def _load_values(self):
        # Set states without triggering callbacks if possible
        self.internal_switch.set_active(self.config.internal_url_enabled)
        self.external_switch.set_active(self.config.external_url_enabled)
        
        self.internal_entry.set_text(self.config.internal_url)
        self.external_entry.set_text(self.config.external_url)
        
        self.internal_entry.set_sensitive(self.config.internal_url_enabled)
        self.external_entry.set_sensitive(self.config.external_url_enabled)
        
        api_key = self.config.get_api_key()
        if api_key:
            self.api_key_entry.set_text(api_key)
            
        # Try to fetch albums asynchronously or synchronously (we'll do sync for simplicity here, but UI might freeze briefly)
        self.remote_albums = self._fetch_remote_albums()
        
        self.folder_rows = [] # keep track of UI objects
        
        for p in self.config.watch_paths:
            if isinstance(p, dict):
                self._add_path_to_ui(p["path"], p.get("album_id"), p.get("album_name"))
            else:
                self._add_path_to_ui(p, None, None)

    def _add_path_to_ui(self, folder, current_album_id=None, current_album_name=None):
        row = Adw.ActionRow(title=folder)
        
        # Album combo box
        model = Gtk.StringList()
        model.append("Default (Folder Name)")
        
        album_names_to_ids = {"Default (Folder Name)": None}
        
        if self.remote_albums:
            for album in self.remote_albums:
                model.append(album['albumName'])
                album_names_to_ids[album['albumName']] = album['id']
                
        # If user had a custom string not in remote somehow
        if current_album_name and current_album_name not in album_names_to_ids:
            model.append(current_album_name)
            album_names_to_ids[current_album_name] = current_album_id

        combo = Gtk.DropDown(model=model)
        combo.set_valign(Gtk.Align.CENTER)
        
        # Set active item
        if current_album_name:
            for i in range(model.get_n_items()):
                if model.get_string(i) == current_album_name:
                    combo.set_selected(i)
                    break
        
        row.add_suffix(combo)
        
        # Remove button
        remove_btn = Gtk.Button(icon_name="user-trash-symbolic")
        remove_btn.set_valign(Gtk.Align.CENTER)
        remove_btn.add_css_class("destructive-action")
        remove_btn.connect("clicked", self.on_remove_folder_clicked, row)
        row.add_suffix(remove_btn)
        
        # Keep track in python list, recreate group
        self.folder_rows.append({"folder": folder, "combo": combo, "row": row, "mapping": album_names_to_ids, "model": model})
        self._refresh_folders_group()

    def _refresh_folders_group(self):
        # Safely remove only the widgets we added
        for widget in self._tracked_group_widgets:
            try:
                self.folders_group.remove(widget)
            except Exception:
                pass
        self._tracked_group_widgets.clear()
        
        # Re-add
        for f in self.folder_rows:
            self.folders_group.add(f["row"])
            self._tracked_group_widgets.append(f["row"])
            
        # Re-add the button at the bottom
        self.folders_group.add(self.add_folder_btn)
        self._tracked_group_widgets.append(self.add_folder_btn)


    def on_add_folder_clicked(self, btn):
        dialog = Gtk.FileDialog()
        dialog.set_title("Select Folder to Watch")
        
        def on_folder_selected(dialog, result):
            try:
                folder = dialog.select_folder_finish(result)
                path = folder.get_path()
                if path:
                    # Check duplicates
                    for f in self.folder_rows:
                        if f["folder"] == path:
                            return
                    self._add_path_to_ui(path)
            except GLib.Error as e:
                logger.error(f"Error selecting folder: {e}")

        dialog.select_folder(self, None, on_folder_selected)

    def on_remove_folder_clicked(self, btn, row):
        for f in self.folder_rows:
            if f["row"] == row:
                self.folder_rows.remove(f)
                break
        self._refresh_folders_group()

    def on_test_connection_clicked(self, btn):
        internal = self.internal_entry.get_text().strip() if self.internal_switch.get_active() else ""
        external = self.external_entry.get_text().strip() if self.external_switch.get_active() else ""
        api_key = self.api_key_entry.get_text().strip()
        
        btn.set_sensitive(False)
        
        def run_test():
            client = ImmichApiClient(internal, external, api_key)
            internal_ok = client._ping(client.internal_url) if internal else False
            external_ok = client._ping(client.external_url) if external else False
            
            GLib.idle_add(show_results, internal_ok, external_ok, client)

        def show_results(internal_ok, external_ok, client):
            btn.set_sensitive(True)
            report = f"Internal Connection: {'OK' if internal_ok else 'FAILED' if internal else 'N/A'}\n"
            report += f"External Connection: {'OK' if external_ok else 'FAILED' if external else 'N/A'}\n"
            
            if internal_ok:
                report += f"\nActive Mode: LAN ({client.internal_url})"
                self._show_msg("Success", report)
            elif external_ok:
                report += f"\nActive Mode: WAN ({client.external_url})"
                self._show_msg("Success", report)
            else:
                report += "\nCould not connect to Immich at either address."
                self._show_msg("Failed", report)
                
            return False # Stop idle
            
        import threading
        threading.Thread(target=run_test, daemon=True).start()

    def on_save_clicked(self, btn):
        logger.info("Saving settings...")
        self.config.data["internal_url"] = self.internal_entry.get_text().strip()
        self.config.data["external_url"] = self.external_entry.get_text().strip()
        self.config.data["internal_url_enabled"] = self.internal_switch.get_active()
        self.config.data["external_url_enabled"] = self.external_switch.get_active()
        
        # Collect paths
        paths = []
        for f in self.folder_rows:
            folder = f["folder"]
            # get currently selected text from drop down
            selected = f["combo"].get_selected()
            album_name = f["model"].get_string(selected)
            album_id = f["mapping"].get(album_name)
            
            paths.append({
                "path": folder,
                "album_id": album_id,
                "album_name": album_name
            })
            
        self.config.data["watch_paths"] = paths
        self.config.save()
        
        key = self.api_key_entry.get_text().strip()
        if key:
            self.config.set_api_key(key)
            
        def on_close(dialog, res):
            self.get_application().quit()
            
        dialog = Adw.MessageDialog(
            transient_for=self,
            heading="Saved",
            body="Settings saved. The application will now exit to restart."
        )
        dialog.add_response("ok", "OK")
        dialog.connect("response", on_close)
        dialog.present()


    def on_about_clicked(self, btn):
        self.show_about_dialog()

    def show_about_dialog(self):
        # Add assets directory to icon theme search path so it can find "icon.png"
        from gi.repository import Gdk
        display = Gdk.Display.get_default()
        if display:
            theme = Gtk.IconTheme.get_for_display(display)
            theme.add_search_path(os.path.join(os.path.dirname(__file__), "assets"))

        about = Adw.AboutWindow(
            application_name="Mimick",
            application_icon="icon",
            version="2.0.0",
            developer_name="Nick Cardoso",
            website="https://github.com/nicx17/mimick",
            issue_url="https://github.com/nicx17/mimick/issues",
            license_type=Gtk.License.GPL_3_0,
            designers=["Nick Cardoso"]
        )
        about.add_link(
            "Logo Illustration by Round Icons",
            "https://unsplash.com/illustrations/a-white-and-orange-flower-on-a-white-background-IkQ_WrJzZOM?utm_source=unsplash&utm_medium=referral&utm_content=creditCopyText"
        )

        about.set_transient_for(self)
        about.present()

    def _show_msg(self, title, msg):
        dialog = Adw.MessageDialog(
            transient_for=self,
            heading=title,
            body=msg
        )
        dialog.add_response("ok", "OK")
        dialog.present()
