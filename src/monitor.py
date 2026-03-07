import time
import os
import logging
import threading
from concurrent.futures import ThreadPoolExecutor
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler
from utils import calculate_checksum
from queue_manager import QueueManager

# Configure logging
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')

# Whitelisted extensions
ALLOWED_EXTENSIONS = {'.jpg', '.jpeg', '.png', '.heic', '.mp4', '.mov', '.gif', '.webp'}

class ImmichEventHandler(FileSystemEventHandler):
    """
    Handles file system events for the Immich Sync App.
    Only processes files with allowed extensions and ignores others (like .xmp sidecars).
    """

    def __init__(self, queue_manager, path_config=None):
        self.queue_manager = queue_manager
        # Mapping mapping from tracked base path -> config dict (e.g. {'album_id': '...', 'album_name': '...'})
        self.path_config = path_config or {}
        # Limit concurrent file checks to avoid thread explosion on bulk copy
        self.executor = ThreadPoolExecutor(max_workers=5, thread_name_prefix="FileScanner")
    
    def on_created(self, event):
        if event.is_directory:
            return
            
        file_path = event.src_path
        _, ext = os.path.splitext(file_path)
        
        # Filter by extension (case-insensitive)
        if ext.lower() not in ALLOWED_EXTENSIONS:
            logging.debug(f"Ignored file (invalid extension): {file_path}")
            return

        logging.info(f"New file change detected: {file_path}")
        # Offload checks to a thread pool to avoid blocking the observer or spawning unlimited threads
        # find corresponding configured path
        matched_config = None
        for base_path, conf in self.path_config.items():
            if os.path.commonpath([str(base_path), str(file_path)]) == str(base_path):
                matched_config = conf
                break
                
        self.executor.submit(self._process_file, file_path, matched_config)

    def on_moved(self, event):
        if event.is_directory:
            return
            
        file_path = event.dest_path
        _, ext = os.path.splitext(file_path)
        
        # Filter by extension (case-insensitive)
        if ext.lower() not in ALLOWED_EXTENSIONS:
            logging.debug(f"Ignored moved file (invalid extension): {file_path}")
            return

        logging.info(f"File moved/renamed detected: {file_path}")
        
        matched_config = None
        for base_path, conf in self.path_config.items():
            if os.path.commonpath([str(base_path), str(file_path)]) == str(base_path):
                matched_config = conf
                break
                
        self.executor.submit(self._process_file, file_path, matched_config)

    def _process_file(self, file_path, path_config=None):
        if self.wait_for_file_completion(file_path):
            checksum = calculate_checksum(file_path)
            if checksum:
                logging.info(f"[FOUND] Valid media file: {file_path}")
                # Add to upload queue
                task = {
                    'path': file_path,
                    'checksum': checksum,
                    'config': path_config
                }
                self.queue_manager.add_to_queue(task)
            else:
                logging.error(f"[ERROR] Could not read file: {file_path}")

    def shutdown(self):
        self.executor.shutdown(wait=True)


    def wait_for_file_completion(self, file_path, idle_timeout=300, check_interval=1.0):
        """
        Wait for the file size to stop changing, indicating it's fully written.
        Uses an idle timeout (default 5 minutes) rather than an absolute timeout.
        As long as the file keeps growing (like a long screen recording), it will keep waiting.
        Requires the file size to remain stable for several consecutive checks.
        """
        last_size = -1
        stable_count = 0
        required_stable_counts = 3  # Must be stable for 3 consecutive checks
        last_change_time = time.time()

        while (time.time() - last_change_time) < idle_timeout:
            try:
                current_size = os.path.getsize(file_path)

                if current_size == last_size and current_size > 0:
                    stable_count += 1
                    if stable_count >= required_stable_counts:
                        return True
                else:
                    stable_count = 0
                    if current_size != last_size:
                        last_change_time = time.time()  # Reset timeout because the file is actively growing
                
                last_size = current_size
            
            except FileNotFoundError:
                return False
            except OSError:
                pass
            
            time.sleep(check_interval)
        
        logging.warning(f"[TIMEOUT] File {file_path} remained locked or inactive for {idle_timeout}s")
        return False

class Monitor:
    def __init__(self, paths_to_watch):
        """
        paths_to_watch: list of strings (paths)
        """
        self.paths_to_watch = paths_to_watch if isinstance(paths_to_watch, list) else [paths_to_watch]
        self.queue_manager = QueueManager()
        self.observer = Observer()

    def start(self, blocking=True):
        self.queue_manager.start()

        path_config_map = {}
        for p in self.paths_to_watch:
             if isinstance(p, dict):
                 path_config_map[p["path"]] = p
             else:
                 path_config_map[p] = {"path": p}

        event_handler = ImmichEventHandler(self.queue_manager, path_config_map)
        
        for p in self.paths_to_watch:
            path = p["path"] if isinstance(p, dict) else p
            if not os.path.exists(path):
                logging.error(f"Error: The directory {path} does not exist. Skipping.")
                continue
                
            self.observer.schedule(event_handler, path, recursive=True)
            logging.info(f"Scheduled watch for: {path}")

        self.observer.start()
        logging.info("Monitor started.")
        
        if blocking:
            try:
                while True:
                    time.sleep(1)
            except KeyboardInterrupt:
                self.stop()
                logging.info("Monitor interrupted by user.")


    def stop(self):
        logging.info("Stopping monitor...")
        self.observer.stop()
        self.queue_manager.stop()
        
        # Shutdown file scanner executor of handler if stored
        # We need access to the handler.
        # But `schedule` does not return the handler object, it returns an Observer object.
        # We need to explicitly track our handler references OR just rely on process exit.
        # Given this is a simple script, process exit cleans up.
        # But to be clean:
        # NOTE: In a real refactor we should store self.event_handlers = []
        
        self.observer.join()

