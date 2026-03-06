import os
import queue
import threading
import time
import logging
import json
from typing import Dict, Any, Optional
from config import Config
from api_client import ImmichApiClient
from notifications import NotificationManager
from state_manager import StateManager

class QueueManager:
    """
    Manages a thread-safe worker queue for processing file uploads.
    """
    def __init__(self):
        self.upload_queue: queue.Queue = queue.Queue()
        self.retry_queue: queue.Queue = queue.Queue() # For failed uploads
        self.stop_event = threading.Event()
        
        # New: Tracking State
        self.notifier = NotificationManager()
        self.state_manager = StateManager()
        self.stats_lock = threading.Lock()
        self.total_queued_session = 0
        self.processed_session = 0
        self.active_workers_count = 0
        
        # Initialize Config and API Client
        self.config = Config()
        api_key = self.config.get_api_key()
        
        if not api_key:
            logging.warning("No API Key found in keyring. Uploads may fail.")
            
        self.api_client = ImmichApiClient(
            self.config.internal_url,
            self.config.external_url,
            api_key if api_key else ""
        )
        
        self.worker_threads = []
        self.num_workers = 10 
        self.retry_storage_file = os.path.expanduser("~/.cache/immich-sync/retries.json")
        self.retry_lock = threading.Lock()
        
    def _save_retries(self):
        with self.retry_lock:
            try:
                items = list(self.retry_queue.queue)
                tmp_file = self.retry_storage_file + f".{threading.get_ident()}.tmp"
                os.makedirs(os.path.dirname(self.retry_storage_file), exist_ok=True)
                with open(tmp_file, 'w') as f:
                    json.dump(items, f)
                os.rename(tmp_file, self.retry_storage_file)
            except Exception as e:
                logging.error(f"Failed to save retries: {e}")
            
    def _load_retries(self):
        try:
            if os.path.exists(self.retry_storage_file):
                with open(self.retry_storage_file, 'r') as f:
                    retries = json.load(f)
                
                for item in retries:
                    self.retry_queue.put(item)
                    
                if retries:
                    logging.info(f"Loaded {len(retries)} items into retry queue from previous session.")
        except Exception as e:
            logging.error(f"Failed to load retries: {e}")

    def _process_retries(self):
        """Periodically check the retry_queue and move items to upload_queue."""
        # Process immediately on cold-boot
        while not self.stop_event.is_set():
            queue_size = self.retry_queue.qsize()
            if queue_size > 0:
                logging.info(f"Re-queuing {queue_size} failed uploads for retry...")
                items_to_retry = []
                while not self.retry_queue.empty():
                    try:
                        item = self.retry_queue.get_nowait()
                        items_to_retry.append(item)
                        self.retry_queue.task_done()
                    except queue.Empty:
                        break
                        
                # Update disk to reflect empty retry_queue
                self._save_retries()
                
                # Push back into upload queue
                for item in items_to_retry:
                    self.add_to_queue(item)
            
            # Wait 60 seconds before checking again (or break if stopping)
            if self.stop_event.wait(timeout=60):
                break

    def start(self):
        logging.info(f"Starting Queue Manager with {self.num_workers} parallel workers...")
        
        # Load any persisted retries before starting threads
        self._load_retries()
        
        for i in range(self.num_workers):
            t = threading.Thread(target=self._process_queue, name=f"Worker-{i+1}", daemon=True)
            self.worker_threads.append(t)
            t.start()
            
        # Start the retry periodic worker
        retry_thread = threading.Thread(target=self._process_retries, name="Retry-Worker", daemon=True)
        self.worker_threads.append(retry_thread)
        retry_thread.start()
        
    def stop(self):
        logging.info("Stopping Queue Manager...")
        self.stop_event.set()
        for t in self.worker_threads:
            t.join(timeout=2)
            
    def _update_stats(self, queued=0, processed=0):
        with self.stats_lock:
            self.total_queued_session += queued
            self.processed_session += processed

    def _publish_state(self, status="idle", current_file=None):
        """Write current state to disk for UI and Notifications"""
        with self.stats_lock:
            real_queue_size = self.upload_queue.qsize()
            
            progress = 0
            if self.total_queued_session > 0:
                progress = min(100, int((self.processed_session / self.total_queued_session) * 100))
                
            state = {
                'queue_size': real_queue_size,
                'total_queued': self.total_queued_session,
                'processed_count': self.processed_session,
                'current_file': current_file,
                'status': status,
                'progress': progress,
                'timestamp': time.time()
            }
        
        self.state_manager.write_state(state)
        return state
        
    def add_to_queue(self, file_info):
        """
        Add a file to the upload queue.
        file_info: dict containing 'path', 'checksum', etc.
        """
        self.upload_queue.put(file_info)
        self._update_stats(queued=1)
        self._publish_state(status="uploading")
        logging.info(f"Queued: {file_info['path']}")

    def _process_queue(self):
        while not self.stop_event.is_set():
            worker_active = False
            try:
                # Wait for items, but timeout occasionally to check stop_event
                file_info = self.upload_queue.get(timeout=1)
                
                # Worker is now active
                with self.stats_lock:
                    self.active_workers_count += 1
                worker_active = True
                
                logging.debug(f"Pop from Queue: {file_info['path']}")
                
                # Notify Start item (Silently update state for UI, suppress desktop notification spam)
                current_state = self._publish_state(status="uploading", current_file=file_info['path'])
                
                # Perform Upload
                t_start = time.time()
                success = False
                try:
                    success = self._handle_upload(file_info)
                except Exception as e:
                    logging.exception(f"Unexpected error in upload handler for {file_info.get('path')}: {e}")
                    success = False
                
                duration = time.time() - t_start
                
                if success:
                    logging.info(f"Upload SUCCESS: {file_info['path']} ({duration:.2f}s)")
                    self._update_stats(processed=1)
                else:
                    logging.warning(f"Upload FAILED: {file_info['path']} ({duration:.2f}s). Re-queuing.")
                    self.retry_queue.put(file_info)
                    logging.info(f"Retry Queue Size: {self.retry_queue.qsize()}")
                    self._save_retries()
                
                self.upload_queue.task_done()
                
                # Worker done with item
                with self.stats_lock:
                    self.active_workers_count -= 1
                worker_active = False
                    
                # Check for "All Done" condition
                if self.upload_queue.empty() and self.active_workers_count == 0:
                    self._publish_state(status="idle", current_file=None)
                    self.notifier.send("Upload Complete", f"Processed {self.processed_session} files.", progress=100)
                else:
                     # Update progress after file completion
                     self._publish_state(status="uploading")
                
            except queue.Empty:
                continue
            except Exception as e:
                # Catch-all strictly for the outer loop to keep the thread alive
                logging.error(f"Error processing queue item: {e}")
                if worker_active:
                    with self.stats_lock:
                        if self.active_workers_count > 0:
                            self.active_workers_count -= 1
                    worker_active = False

    def _handle_upload(self, file_info):
        """
        Uploads the file using ApiClient and adds to album.
        """
        file_path = file_info['path']
        checksum = file_info['checksum']
        
        # 1. Upload Asset
        asset_id = self.api_client.upload_asset(file_path, checksum)
        
        if asset_id is None: # Explicitly check None, False, or string
            return False

        if asset_id == "DUPLICATE":
            logging.info(f"Asset already exists on server: {file_path}")
            return True

        # 2. Determine Album Name and ID
        config = file_info.get('config', {})
        album_id = config.get('album_id')
        album_name = config.get('album_name')
        
        # Determine fallback name if no explicit config provided
        if not album_id and (not album_name or album_name == "Default (Folder Name)"):
            parent_dir = os.path.basename(os.path.dirname(file_path))
            album_name = parent_dir
            
        logging.info(f"Preparing to add '{file_path}' to album '{album_name}' (ID: {album_id})")
        
        try:
            # 3. Get or Create Album if ID isn't known
            if not album_id:
                album_id = self.api_client.get_or_create_album(album_name)
            
            if album_id:
                # 4. Add to Album
                self.api_client.add_assets_to_album(album_id, [asset_id])
            else:
                logging.warning(f"Could not get/create album '{album_name}'")
        except Exception as e:
            logging.error(f"Error during album assignment: {e}")
        
        return True
