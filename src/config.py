import os
import json
import keyring
import logging

APP_NAME = "immich-sync"
CONFIG_DIR = os.path.join(os.path.expanduser("~"), ".config", APP_NAME)
CONFIG_FILE = os.path.join(CONFIG_DIR, "config.json")

class Config:
    def __init__(self):
        self._ensure_config_dir()
        self.data = self._load_config()

    def _ensure_config_dir(self):
        if not os.path.exists(CONFIG_DIR):
            os.makedirs(CONFIG_DIR, exist_ok=True)
            
    def _load_config(self):
        logging.debug(f"Loading config from {CONFIG_FILE}")
        if not os.path.exists(CONFIG_FILE):
             logging.info("Config file not found, creating default.")
             # Default structure
             return {
                 "watch_paths": [os.path.join(os.path.expanduser("~"), "Pictures")],
                 "internal_url": "http://immich-server:2283",
                 "external_url": "https://immich.example.com",
                 "internal_url_enabled": True,
                 "external_url_enabled": True
             }
             
        try:
            with open(CONFIG_FILE, 'r') as f:
                data = json.load(f)
                logging.debug("Config loaded successfully.")
                return data
        except json.JSONDecodeError as e:
            logging.error(f"Failed to load config (JSON error): {e}")
            return {}

    def save(self):
        with open(CONFIG_FILE, 'w') as f:
            json.dump(self.data, f, indent=4)
            
    def get_api_key(self):
        try:
            return keyring.get_password(APP_NAME, "api_key")
        except Exception as e:
            logging.error(f"Keyring error: {e}")
            return None
        
    def set_api_key(self, api_key):
        try:
            keyring.set_password(APP_NAME, "api_key", api_key)
        except Exception as e:
            logging.error(f"Keyring error: {e}")
            
    # Properties for easy access
    @property
    def internal_url_enabled(self):
        return self.data.get("internal_url_enabled", True)
        
    @property
    def external_url_enabled(self):
        return self.data.get("external_url_enabled", True)
        
    @property
    def internal_url(self):
        return self.data.get("internal_url", "")
    
    @property
    def external_url(self):
        return self.data.get("external_url", "")
    
    @property
    def watch_paths(self):
        paths = self.data.get("watch_paths", [])
        # Normalise to list of dicts if they are just strings (backwards compatibility)
        normalized = []
        for p in paths:
            if isinstance(p, str):
                normalized.append({"path": p, "album_id": None, "album_name": ""})
            elif isinstance(p, dict):
                normalized.append(p)
        return normalized
