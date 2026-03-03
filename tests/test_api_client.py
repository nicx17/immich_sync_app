import os
import sys
import io
import requests
import requests_mock
import pytest
from unittest.mock import MagicMock, patch

# Add src to path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..', 'src')))

from api_client import ImmichApiClient

@pytest.fixture
def client():
    return ImmichApiClient("http://lan:2283", "https://wan.example.com", "fake-key")

def test_ping_success(client):
    """Test successful ping."""
    with requests_mock.Mocker() as m:
        m.get("http://lan:2283/api/server/ping", text="pong", status_code=200)
        assert client._ping("http://lan:2283") is True

def test_ping_failure(client):
    """Test ping failure (404/500)."""
    with requests_mock.Mocker() as m:
        m.get("http://lan:2283/api/server/ping", text="error", status_code=500)
        assert client._ping("http://lan:2283") is False

def test_check_connection_lan_preferred(client):
    """Test that LAN is chosen first if available."""
    with requests_mock.Mocker() as m:
        m.get("http://lan:2283/api/server/ping", text="pong", status_code=200)
        
        # WAN might also work, but shouldn't be called if LAN works
        m.get("https://wan.example.com/api/server/ping", text="pong", status_code=200)
        
        client.check_connection()
        assert client.active_url == "http://lan:2283"

def test_check_connection_wan_fallback(client):
    """Test fallback to WAN if LAN fails."""
    with requests_mock.Mocker() as m:
        m.get("http://lan:2283/api/server/ping", exc=requests.exceptions.ConnectTimeout)
        m.get("https://wan.example.com/api/server/ping", text="pong", status_code=200)
        
        client.check_connection()
        assert client.active_url == "https://wan.example.com"

def test_upload_asset_success(client):
    """Test asset upload logic."""
    client.active_url = "http://immich:2283"
    file_path = "/tmp/test.jpg"
    checksum = "abc12345"
    
    with requests_mock.Mocker() as m:
        m.post("http://immich:2283/api/assets", text='{"id": "123"}', status_code=201)
        
        # We need a file-like object that requests can read from.
        # MagicMock isn't enough because it doesn't behave like a byte stream.
        mock_file_content = io.BytesIO(b"fake image data")

        # Patch 'builtins.open' so that when it's called, it returns our mock_file_content
        with patch('os.stat') as mock_stat, \
             patch('builtins.open', return_value=mock_file_content), \
             patch('requests.sessions.get_netrc_auth', return_value=None):
            
            # Setup file stat mock
            mock_stat.return_value.st_ctime = 1672531200 # 2023-01-01
            mock_stat.return_value.st_mtime = 1672531200
            
            # Execute
            result = client.upload_asset(file_path, checksum)
            assert result == "123"
            
            # Verify request
            assert m.called
            assert m.last_request.headers['x-api-key'] == 'fake-key'
            # Check content length matches fake data length (15 bytes)
            # requests might add some overhead for multipart/form-data boundaries, so > 15
            assert int(m.last_request.headers['Content-Length']) > 15 

def test_upload_asset_duplicate(client):
    """Test handling of 409 Conflict (Duplicate)."""
    client.active_url = "http://immich:2283"
    
    with requests_mock.Mocker() as m:
        m.post("http://immich:2283/api/assets", status_code=409)
        
        mock_file_content = io.BytesIO(b"duplicate data")
        
        with patch('os.stat') as mock_stat, \
             patch('builtins.open', return_value=mock_file_content), \
             patch('requests.sessions.get_netrc_auth', return_value=None):
             
            mock_stat.return_value.st_ctime = 1672531200
            mock_stat.return_value.st_mtime = 1672531200
            
            result = client.upload_asset("/tmp/dupe.jpg", "hash")
            assert result == "DUPLICATE"

def test_upload_asset_server_error(client):
    """Test handling of 502 Bad Gateway."""
    client.active_url = "http://immich:2283"
    
    with requests_mock.Mocker() as m:
        m.post("http://immich:2283/api/assets", status_code=502)
        
        mock_file_content = io.BytesIO(b"error data")

        with patch('os.stat') as mock_stat, \
             patch('builtins.open', return_value=mock_file_content), \
             patch('requests.sessions.get_netrc_auth', return_value=None):
             
            mock_stat.return_value.st_ctime = 1672531200
            mock_stat.return_value.st_mtime = 1672531200
            
            result = client.upload_asset("/tmp/error.jpg", "hash")
            assert result is None # Retry needed
