import os
import sys
import json
import pytest
from unittest.mock import MagicMock, patch, mock_open

# Add src to path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from config import Config, CONFIG_FILE  # noqa: E402

@pytest.fixture
def mock_keyring():
    with patch('keyring.get_password') as mock_get, \
         patch('keyring.set_password') as mock_set:
        yield mock_get, mock_set

def test_config_load_defaults():
    """Test loading defaults when config file doesn't exist."""
    with patch('os.path.exists', return_value=False), \
         patch('os.makedirs'):
        
        config = Config()
        
        assert config.data["internal_url"] == "http://immich-server:2283"
        assert config.data["external_url"] == "https://immich.example.com"
        assert "watch_paths" in config.data

def test_config_load_existing():
    """Test loading from an existing JSON file."""
    fake_config = {
        "watch_paths": ["/home/user/TEST"],
        "internal_url": "http://1.2.3.4",
        "external_url": "https://test.com"
    }
    
    with patch('os.path.exists', return_value=True), \
         patch('builtins.open', mock_open(read_data=json.dumps(fake_config))), \
         patch('json.load', return_value=fake_config):
         
        config = Config()
        
        assert config.data["watch_paths"] == ["/home/user/TEST"]
        assert config.internal_url == "http://1.2.3.4"

def test_config_load_corrupt():
    """Test loading a corrupt JSON config falls back to default empty dict."""
    with patch('os.path.exists', return_value=True), \
         patch('builtins.open', mock_open(read_data="{invalid_json")), \
         patch('json.load', side_effect=json.JSONDecodeError("Expecting value", "", 0)):
         
        config = Config()
        
        # In current logic, corrupt or un-decodable JSON returns an empty dict
        assert config.data == {}
        assert config.internal_url == ""

def test_config_save():
    """Test saving configuration updates to file."""
    with patch('os.path.exists', return_value=False), \
         patch('os.makedirs'), \
         patch('builtins.open', mock_open()) as mock_file:
         
        config = Config()
        config.data["internal_url"] = "http://updated.local"
        config.save()
        
        mock_file.assert_called_with(CONFIG_FILE, 'w')
        # Check if write was called (serialization check)
        handle = mock_file()
        handle.write.assert_called()

def test_get_api_key(mock_keyring):
    """Test retrieving API key from system keyring."""
    mock_get, _ = mock_keyring
    mock_get.return_value = "secret-key-123"
    
    with patch('os.path.exists', return_value=False), patch('os.makedirs'):
        config = Config()
        key = config.get_api_key()
        
        assert key == "secret-key-123"
        mock_get.assert_called_with("mimick", "api_key")

def test_get_api_key_exception(mock_keyring):
    """Test getting API key when keyring throws an exception."""
    mock_get, _ = mock_keyring
    mock_get.side_effect = Exception("No keyring backend")
    
    with patch('os.path.exists', return_value=False), patch('os.makedirs'):
        config = Config()
        assert config.get_api_key() is None

def test_set_api_key(mock_keyring):
    """Test saving API key to system keyring."""
    _, mock_set = mock_keyring
    
    with patch('os.path.exists', return_value=False), patch('os.makedirs'):
        config = Config()
        config.set_api_key("new-secret")
        
        mock_set.assert_called_with("mimick", "api_key", "new-secret")

def test_set_api_key_exception(mock_keyring):
    """Test saving API key when keyring throws an exception."""
    _, mock_set = mock_keyring
    mock_set.side_effect = Exception("No keyring backend")
    
    with patch('os.path.exists', return_value=False), patch('os.makedirs'):
        config = Config()
        # Should catch and not raise
        config.set_api_key("new-secret")

def test_config_properties():
    """Test the computed properties map correctly."""
    fake_config = {
        "watch_paths": ["/string/path", {"path": "/dict/path", "album_id": "123", "album_name": "Test"}],
        "internal_url_enabled": False,
        "external_url_enabled": True
    }
    with patch('os.path.exists', return_value=True), \
         patch('builtins.open', mock_open(read_data=json.dumps(fake_config))), \
         patch('json.load', return_value=fake_config):
         
        config = Config()
        assert config.internal_url_enabled is False
        assert config.external_url_enabled is True
        
        # Test watch_paths backwards compatibility normalisation
        paths = config.watch_paths
        assert len(paths) == 2
        assert paths[0] == {"path": "/string/path", "album_id": None, "album_name": ""}
        assert paths[1] == {"path": "/dict/path", "album_id": "123", "album_name": "Test"}

