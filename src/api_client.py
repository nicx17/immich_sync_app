import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry
import logging
import time
import os
from typing import Optional, Dict
from datetime import datetime, timezone

class ImmichApiClient:

    def __init__(self, internal_url: str, external_url: str, api_key: str):
        self.internal_url = internal_url.rstrip('/')
        self.external_url = external_url.rstrip('/')
        self.api_key = api_key
        self.active_url: Optional[str] = None
        self.album_cache: Dict[str, str] = {}  # Cache name->id
        self.albums_fetched = False
        
        # Configure a robust session with connection pooling
        self.session = requests.Session()
        pool_size = 20 # Allow high concurrency
        retries = Retry(total=3, backoff_factor=0.5, status_forcelist=[500, 502, 503, 504])
        adapter = HTTPAdapter(pool_connections=pool_size, pool_maxsize=pool_size, max_retries=retries)
        self.session.mount('http://', adapter)
        self.session.mount('https://', adapter)
        
    def check_connection(self) -> bool:
        """
        Determines which URL to use (Internal vs External).
        Checks Internal first.
        """
        logging.info("Checking connectivity...")
        
        # Try Internal First
        if self._ping(self.internal_url):
            self.active_url = self.internal_url
            logging.info(f"Connected via LAN: {self.active_url}")
            return True
            
        # Try External
        if self._ping(self.external_url):
            self.active_url = self.external_url
            logging.info(f"Connected via WAN: {self.active_url}")
            return True
            
        logging.error("Could not connect to Immich Server.")
        self.active_url = None
        return False

    def _ping(self, url: str) -> bool:
        """
        Pings the server's /server/ping endpoint.
        """
        if not url: return False
        
        target = f"{url}/api/server/ping"
        logging.debug(f"Pinging: {target}")
        try:
            # Short timeout for the ping
            response = self.session.get(target, timeout=2)
            if response.status_code == 200:
                try:
                    data = response.json()
                    if data.get("res", "").lower() == "pong":
                        logging.debug(f"Ping Success: {target}")
                        return True
                except ValueError:
                    pass
                
                logging.warning(f"Ping Failed (Not a valid Immich response): {target}")
                return False
            else:
                logging.warning(f"Ping Failed ({response.status_code}): {target}")
                return False
        except requests.RequestException as e:
            logging.warning(f"Ping Error ({e}): {target}")
            return False

    def check_asset_exists(self, checksum: str) -> bool:
        """
        Checks if an asset with the given SHA-1 checksum already exists on the server.
        NOTE: This is currently a placeholder as the specific check API endpoint depends on Immich version.
        Optimistic upload (upload and handle 409 Conflict) is often more reliable.
        """
        return False 
        
    def upload_asset(self, file_path: str, checksum: str) -> Optional[str]:
        """
        Uploads the asset with strict ISO 8601 timestamps.
        Returns: Asset ID (String) if successful, None if Failed/Retry
        """
        if not self.active_url and not self.check_connection():
            logging.error("No active connection to upload.")
            return None

        # Updated endpoint to match modern Immich API (v1.118+)
        # Reference: https://api.immich.app/endpoints/assets/uploadAsset -> POST /assets
        url = f"{self.active_url}/api/assets"
        
        headers = {
            "x-api-key": self.api_key,
            "Accept": "application/json"
        }
        
        # Get file metadata
        try:
            file_stats = os.stat(file_path)
            # Strictly Format timestamps as ISO 8601 UTC
            created_at = datetime.fromtimestamp(file_stats.st_ctime, timezone.utc).isoformat()
            modified_at = datetime.fromtimestamp(file_stats.st_mtime, timezone.utc).isoformat()
        except OSError:
            logging.error(f"Could not read stats for {file_path}")
            return None
            
        device_asset_id = f"python-sync-{checksum}" # Unique ID
        device_id = "python-sync-client"
        
        data = {
            "deviceAssetId": device_asset_id,
            "deviceId": device_id,
            "fileCreatedAt": created_at,
            "fileModifiedAt": modified_at,
            "isFavorite": "false",
        }
        
        try:
            logging.info(f"Start Upload: {file_path} ({file_stats.st_size} bytes)")
            logging.debug(f"Upload Endpoint: {url}")
            logging.debug(f"Metadata: AssetID={device_asset_id}, Created={created_at}")

            with open(file_path, "rb") as f:
                # Use a specific mime-type if possible, but octet-stream is safe fallback
                files = {
                    "assetData": (os.path.basename(file_path), f, "application/octet-stream")
                }
                
                response = self.session.post(url, headers=headers, data=data, files=files, timeout=30)
            
            logging.info(f"Upload Completed: Status {response.status_code}")

            if response.status_code in [200, 201]:
                res_json = response.json()
                asset_id = res_json.get("id")
                logging.info(f"Upload Success: {file_path} (ID: {asset_id})")
                return asset_id
            elif response.status_code == 409:
                logging.info(f"Upload Skipped (Already Exists): {file_path}")
                # For 409, we might get the ID in response depending on version, 
                # but often it just says 'Duplicate'. 
                # Ideally we fetch it if we want to add to album, but let's check response
                try:
                    res_json = response.json()
                    # Some versions return { id: "...", duplicate: true }
                    if "id" in res_json:
                        return res_json["id"]
                except:
                    pass
                return "DUPLICATE" # Special marker if we can't get ID
            elif response.status_code in [413]:
                logging.error(f"Upload Failed (Too Large): {file_path}")
                return None
            elif response.status_code in [502, 504]:
                logging.warning(f"Server Error {response.status_code}: Retrying later...")
                return None
            else:
                logging.error(f"Upload Failed {response.status_code}: {response.text}")
                return None

        except requests.RequestException as e:
            logging.error(f"Network Error during upload: {e}")
            self.active_url = None  # Force re-connection on next try
            return None

    def get_albums(self):
        """
        Returns a list of dictionaries with 'id' and 'albumName' 
        for UI presentation.
        """
        if not self.albums_fetched:
            self._fetch_all_albums()
            
        if not self.active_url or not self.albums_fetched:
             return []
             
        # Return directly from cache, recreating the map structure for UI
        return [{"id": v, "albumName": k} for k, v in self.album_cache.items()]

    def _fetch_all_albums(self):
        """
        Fetches all owned albums.
        """
        if not self.active_url: 
             logging.warning("Cannot fetch albums: No active URL.")
             return
        
        # NOTE: /api/albums usually returns all user albums.
        # We might want to filter shared ones if needed, but for now we take all.
        url = f"{self.active_url}/api/albums"
        
        # Try both endpoints since API changes frequently
        # v1.118+ is /api/albums, older might differ but usually /api/album
        
        headers = {
            "x-api-key": self.api_key, 
            "Accept": "application/json"
        }
        
        try:
            logging.info("Fetching existing albums list...")
            response = self.session.get(url, headers=headers, timeout=10)
            
            if response.status_code == 200:
                albums = response.json()
                # Store Map: Name -> ID
                # If multiple albums show same name, last one wins (acceptable limitation)
                self.album_cache = {a['albumName']: a['id'] for a in albums}
                self.albums_fetched = True
                logging.info(f"Cached {len(self.album_cache)} albums.")
            else:
                logging.error(f"Failed to fetch albums: {response.status_code} {response.text}")
        except requests.RequestException as e:
            logging.error(f"Network error fetching albums: {e}")
            self.active_url = None
        except Exception as e:
            logging.error(f"Error fetching albums: {e}")

    def create_album(self, album_name):
        """
        Creates a new album.
        """
        if not self.active_url: return None
        
        url = f"{self.active_url}/api/albums"
        headers = {
            "x-api-key": self.api_key, 
            "Content-Type": "application/json",
            "Accept": "application/json"
        }
        
        data = {
            "albumName": album_name, 
            "description": "Created by Immich Auto-Sync"
        }
        
        try:
            logging.info(f"Creating new album: '{album_name}'")
            response = self.session.post(url, headers=headers, json=data, timeout=10)
            if response.status_code in [200, 201]:
                res_json = response.json()
                album_id = res_json['id']
                self.album_cache[album_name] = album_id # Update cache
                logging.info(f"Album created: {album_name} ({album_id})")
                return album_id
            else:
                logging.error(f"Failed to create album: {response.status_code} {response.text}")
                return None
        except requests.RequestException as e:
            logging.error(f"Network error creating album: {e}")
            self.active_url = None
            return None
        except Exception as e:
            logging.error(f"Error creating album: {e}")
            return None

    def get_or_create_album(self, album_name):
        """
        Returns album ID for the given name, creating it if necessary.
        """
        # Ensure we have the cache populated at least once
        if not self.albums_fetched:
            self._fetch_all_albums()
            
        # Check cache
        if album_name in self.album_cache:
            logging.debug(f"Album found in cache: {album_name}")
            return self.album_cache[album_name]
            
        # If not found, try to create
        # (Alternatively, we could re-fetch to be sure, but let's assume cache is mostly up to date)
        # We might re-fetch if creation returns 409 (Conflict)? But album names aren't unique in Immich.
        # Immich allows duplicate album names. We just create a new one if we don't know it.
        return self.create_album(album_name)

    def add_assets_to_album(self, album_id, asset_ids):
        """
        Adds a list of asset IDs to an album.
        """
        if not self.active_url or not album_id or not asset_ids: 
             logging.warning("Skipping add_assets_to_album: Missing ID or Assets.")
             return False
        
        url = f"{self.active_url}/api/albums/{album_id}/assets"
        headers = {
            "x-api-key": self.api_key, 
            "Content-Type": "application/json",
            "Accept": "application/json"
        }
        
        data = {"ids": asset_ids}
        
        try:
            logging.info(f"Adding assets {asset_ids} to album {album_id}...")
            response = self.session.put(url, headers=headers, json=data, timeout=10)
            
            # Response is a list of results
            if response.status_code in [200, 201]:
                logging.info(f"Successfully added assets to album.")
                return True
            else:
                logging.error(f"Failed to add assets to album: {response.status_code} {response.text}")
                return False
        except requests.RequestException as e:
            logging.error(f"Network error adding assets to album: {e}")
            self.active_url = None
            return False
        except Exception as e:
            logging.error(f"Error adding assets to album: {e}")
            return False

