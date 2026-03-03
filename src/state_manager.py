import json
import os
import time

class StateManager:
    def __init__(self):
        # Store state in XDG Runtime Dir or Cache. 
        # Using ~/.cache/immich-sync/status.json is generally safe and persistent enough for this.
        self.state_file = os.path.expanduser("~/.cache/immich-sync/status.json")
        self._ensure_dir()
        
    def _ensure_dir(self):
        os.makedirs(os.path.dirname(self.state_file), exist_ok=True)
        
    def write_state(self, state):
        """
        Write state dict to JSON file.
        state: {
            'queue_size': int,
            'total_queued': int,
            'processed_count': int,
            'current_file': str,
            'status': 'idle' | 'uploading' | 'error',
            'timestamp': float
        }
        """
        try:
            # Atomic write pattern to avoid partial reads by the UI
            tmp_file = self.state_file + ".tmp"
            with open(tmp_file, 'w') as f:
                json.dump(state, f)
            os.rename(tmp_file, self.state_file)
        except Exception as e:
            pass
            
    def read_state(self):
        try:
            if not os.path.exists(self.state_file):
                return {}
            with open(self.state_file, 'r') as f:
                return json.load(f)
        except (FileNotFoundError, json.JSONDecodeError, OSError):
            return {}
