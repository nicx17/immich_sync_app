import os
import queue
import threading
import time
import logging
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
        
    def start(self):
        logging.info(f"Starting Queue Manager with {self.num_workers} parallel workers...")
        for i in range(self.num_workers):
            t = threading.Thread(target=self._process_queue, name=f"Worker-{i+1}", daemon=True)
            self.worker_threads.append(t)
            t.start()
        
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
                
                # Notify Start item
                current_state = self._publish_state(status="uploading", current_file=file_info['path'])
                
                self.notifier.send(
                    f"Uploading ({current_state['progress']}%)", 
                    f"Processing: {os.path.basename(file_info['path'])}",
                    progress=current_state['progress']
                )
                
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

        # 2. Determine Album Name
        # Use parent folder name relative to watch path root would be ideal, 
        # but for now just immediate parent directory name is simple and effective.
        parent_dir = os.path.basename(os.path.dirname(file_path))
        album_name = parent_dir
        
        logging.info(f"Preparing to add '{file_path}' to album '{album_name}'")
        
        try:
            # 3. Get or Create Album
            album_id = self.api_client.get_or_create_album(album_name)
            
            if album_id:
                # 4. Add to Album
                self.api_client.add_assets_to_album(album_id, [asset_id])
            else:
                logging.warning(f"Could not get/create album '{album_name}'")
        except Exception as e:
            logging.error(f"Error during album assignment: {e}")
        
        return True
