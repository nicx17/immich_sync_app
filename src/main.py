import argparse
import sys
import os
import logging
import signal

# Force compatibility backend for GNOME/Wayland Tray Icons
# 'Unity' forces usage of AppIndicator3 over legacy GtkStatusIcon
os.environ["XDG_CURRENT_DESKTOP"] = "Unity"
# Force GDK to use X11 backend (XWayland) to avoid GTK Wayland scaling bugs
os.environ["GDK_BACKEND"] = "x11"

from monitor import Monitor
from config import Config

# Configure logging
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')

def main():
    parser = argparse.ArgumentParser(description="Immich Sync - Linux Background Daemon")
    parser.add_argument("path", nargs="?", help="Override watch path (optional)", default=None)
    parser.add_argument("--no-tray", action="store_true", help="Run in terminal mode without system tray")
    args = parser.parse_args()

    # Load configuration
    config = Config()
    logging.info(f"Loaded configuration: {config.data}")
    
    # determine paths to watch
    watch_paths = []
    
    if args.path:
        logging.info(f"Using CLI provided path: {args.path}")
        watch_paths = [args.path]
    else:
        watch_paths = config.data.get("watch_paths", [])
        logging.info(f"Using configured watch paths: {watch_paths}")
        
    # fallback if config is empty
    if not watch_paths:
        user_home = os.path.expanduser("~")
        potential_path = os.path.join(user_home, "Pictures")
        if os.path.exists(potential_path):
             logging.warning(f"No paths configured. Falling back to default Pictures folder: {potential_path}")
             watch_paths = [potential_path]
        else:
            logging.critical("No valid watch paths found from CLI or Config.")
            print("Please provide a path to watch either via CLI or config.json.")
            sys.exit(1)

    logging.info(f"Immich Auto-Sync starting with paths: {watch_paths}")
    monitor = Monitor(watch_paths)
    
    if args.no_tray:
        # Run in blocking mode (terminal only)
        logging.info("Running in headless/terminal mode.")
        monitor.start(blocking=True)
    else:
        # Run in Daemon Mode (pystray only)
        # The GUI (Settings) is launched as a subprocess
        from tray_icon import TrayIcon

        # Start Monitor in non-blocking mode (background threads)
        logging.info("Starting file monitor in background thread...")
        monitor.start(blocking=False)
        
        # Initialize Tray
        logging.info("Initializing system tray icon...")
        tray = TrayIcon(monitor)
        
        # Run Tray (Blocking Main Thread)
        # Pystray uses GTK/AppIndicator binding which prefers being main loop
        logging.info("Entering main loop (Tray)...")
        tray.run()



if __name__ == "__main__":
    main()
