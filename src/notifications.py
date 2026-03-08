import subprocess
import logging
logger = logging.getLogger(__name__)

class NotificationManager:
    def __init__(self, app_name="Immich Auto-Sync"):
        self.app_name = app_name
        self.notification_id = "immich-sync-progress" # Unique ID for replacement

    def send(self, title, message, progress=None, timeout=None):
        """
        Send a notification using notify-send.
        progress: int (0-100) or None
        timeout: int (milliseconds) or None
        """
        cmd = ["notify-send", "--app-name", self.app_name, title, message]
        
        # Use synchronous hint to replace existing notification (progress bar effect)
        # This ID links notifications so they update in-place
        cmd.extend(["-h", f"string:x-canonical-private-synchronous:{self.notification_id}"])
        
        if progress is not None:
            # Standard hint for progress value on many Linux DEs (KDE, GNOME)
            cmd.extend(["-h", f"int:value:{int(progress)}"])
            
        if timeout:
            cmd.extend(["-t", str(timeout)])
            
        try:
            subprocess.Popen(cmd)
        except FileNotFoundError:
            # Only log once or just suppress to avoid log spam if not installed
            pass
        except Exception as e:
            logger.error(f"Failed to send notification: {e}")
