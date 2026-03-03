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

    def __init__(self, queue_manager):
        self.queue_manager = queue_manager
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
        self.executor.submit(self._process_file, file_path)

    def _process_file(self, file_path):
        if self.wait_for_file_completion(file_path):
            checksum = calculate_checksum(file_path)
            if checksum:
                logging.info(f"[FOUND] Valid media file: {file_path}")
                # Add to upload queue
                task = {
                    'path': file_path,
                    'checksum': checksum
                }
                self.queue_manager.add_to_queue(task)
            else:
                logging.error(f"[ERROR] Could not read file: {file_path}")

    def shutdown(self):
        self.executor.shutdown(wait=True)


    def wait_for_file_completion(self, file_path, timeout=10, check_interval=0.1):
        """
        Wait for the file size to stop changing.
        Returns True if the file is ready, False on timeout or error.
        Check interval reduced to 0.1s for responsiveness.
        """
        last_size = -1
        start_time = time.time()

        while (time.time() - start_time) < timeout:
            try:
                current_size = os.path.getsize(file_path)

                if current_size == last_size and current_size > 0:
                    return True
                
                last_size = current_size
                time.sleep(check_interval)
            
            except FileNotFoundError:
                return False
            except OSError:
                pass
        
        logging.warning(f"[TIMEOUT] File {file_path} unstable for {timeout}s")
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

        event_handler = ImmichEventHandler(self.queue_manager) # Pass queue manager
        
        for path in self.paths_to_watch:
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

