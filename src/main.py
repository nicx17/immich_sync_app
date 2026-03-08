import argparse
import sys
import os
import logging
logger = logging.getLogger(__name__)
import signal
import socket
import subprocess

from log_setup import setup_logging
setup_logging()

# Force compatibility backend for GNOME/Wayland Tray Icons
# 'Unity' forces usage of AppIndicator3 over legacy GtkStatusIcon
os.environ["XDG_CURRENT_DESKTOP"] = "Unity"
# Force GDK to use X11 backend (XWayland) to avoid GTK Wayland scaling bugs
os.environ["GDK_BACKEND"] = "x11"

from monitor import Monitor
from config import Config

def check_single_instance_or_run_settings(args):
    """
    Ensures only one daemon runs. If another is already running, it forwards
    any relevant commands (like opening settings).
    """
    lock_socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        lock_socket.bind('\0immich-sync-daemon-lock')
        
        # If we got the lock, and they specifically ONLY asked for settings,
        # we can just launch settings_main.py and keep daemon running if we want,
        # but normally if they ask for settings and daemon isn't running, we just run daemon
        # and open settings.
        if args.settings:
            script_path = os.path.join(os.path.dirname(__file__), "settings_main.py")
            subprocess.Popen([sys.executable, script_path])
        return lock_socket
    except socket.error:
        logger.info("Immich Auto-Sync daemon is already running.")
        # If daemon is running and user clicked launcher, open settings.
        script_path = os.path.join(os.path.dirname(__file__), "settings_main.py")
        print("Daemon is already running. Opening settings window instead.")
        proc = subprocess.Popen([sys.executable, script_path])
        proc.wait() # Block until the settings window closes or forwards its request
        sys.exit(0)

def main():
    parser = argparse.ArgumentParser(description="Immich Sync - Linux Background Daemon")
    parser.add_argument("path", nargs="?", help="Override watch path (optional)", default=None)
    parser.add_argument("--no-tray", action="store_true", help="Run in terminal mode without system tray")
    parser.add_argument("--settings", action="store_true", help="Launch settings window directly")
    args = parser.parse_args()

    _lock = check_single_instance_or_run_settings(args)

    # Load configuration
    config = Config()
    logger.info(f"Loaded configuration: {config.data}")
    
    # determine paths to watch
    watch_paths = []
    
    if args.path:
        logger.info(f"Using CLI provided path: {args.path}")
        watch_paths = [args.path]
    else:
        watch_paths = config.data.get("watch_paths", [])
        logger.info(f"Using configured watch paths: {watch_paths}")
        
    # fallback if config is empty
    if not watch_paths:
        user_home = os.path.expanduser("~")
        potential_path = os.path.join(user_home, "Pictures")
        if os.path.exists(potential_path):
             logger.warning(f"No paths configured. Falling back to default Pictures folder: {potential_path}")
             watch_paths = [potential_path]
        else:
            logger.critical("No valid watch paths found from CLI or Config.")
            print("Please provide a path to watch either via CLI or config.json.")
            sys.exit(1)

    logger.info(f"Immich Auto-Sync starting with paths: {watch_paths}")
    monitor = Monitor(watch_paths)
    
    if args.no_tray:
        # Run in blocking mode (terminal only)
        logger.info("Running in headless/terminal mode.")
        monitor.start(blocking=True)
    else:
        # Run in Daemon Mode (pystray only)
        # The GUI (Settings) is launched as a subprocess
        
        # Start Monitor in non-blocking mode (background threads)
        logger.info("Starting file monitor in background thread...")
        monitor.start(blocking=False)
        
        try:
            from tray_icon import TrayIcon
            
            # Initialize Tray
            logger.info("Initializing system tray icon...")
            tray = TrayIcon(monitor)
            
            # Run Tray (Blocking Main Thread)
            # Pystray uses GTK/AppIndicator binding which prefers being main loop
            logger.info("Entering main loop (Tray)...")
            tray.run()
            
            if not tray._exiting_cleanly:
                raise RuntimeError("Tray icon exited unexpectedly (failed to attach or crashed).")
                
        except Exception as e:
            logger.error(f"Failed to initialize system tray icon: {e}")
            logger.info("Tray icon feature disabled due to OS configuration or error.")
            logger.info("Running daemon in background (headless config).")
            
            # Since the tray failed to load, open the settings window if this was a direct launch 
            # so the user knows the app started (unless they explicitly passed --settings, though that's handled above).
            script_path = os.path.join(os.path.dirname(__file__), "settings_main.py")
            subprocess.Popen([sys.executable, script_path])
            
            try:
                import time
                while True:
                    time.sleep(1)
            except KeyboardInterrupt:
                logger.info("Shutting down daemon...")
                monitor.stop()

if __name__ == "__main__":
    main()
