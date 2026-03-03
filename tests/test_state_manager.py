import os
import sys
import json
import pytest
from unittest.mock import patch

# Add src to path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from state_manager import StateManager

@pytest.fixture
def temp_state_manager(tmp_path):
    # Mock expanduser inside the class initialization to place state_file in tmp_path
    with patch('os.path.expanduser', return_value=os.path.join(tmp_path, ".cache/immich-sync/status.json")):
        manager = StateManager()
        yield manager

def test_init_creates_directory(temp_state_manager):
    # Ensure initialization creates the directory structure
    assert os.path.exists(os.path.dirname(temp_state_manager.state_file))

def test_write_and_read_state_success(temp_state_manager):
    test_state = {
        'status': 'uploading',
        'progress': 80,
        'current_file': 'img.jpg'
    }
    
    # Write
    temp_state_manager.write_state(test_state)
    
    # Check if the final file exists (and tmp file was renamed)
    assert os.path.exists(temp_state_manager.state_file)
    assert not os.path.exists(temp_state_manager.state_file + ".tmp")
    
    # Read
    read_data = temp_state_manager.read_state()
    assert read_data == test_state

def test_write_state_exception(temp_state_manager):
    # Force an exception by mocking 'open'
    test_state = {'test': 1}
    with patch('builtins.open', side_effect=Exception("Permission Denied")):
        # Should not raise
        temp_state_manager.write_state(test_state)
        
    assert not os.path.exists(temp_state_manager.state_file)

def test_read_state_not_exists(temp_state_manager):
    # File does not exist yet
    assert not os.path.exists(temp_state_manager.state_file)
    
    # Should safely return empty dict
    result = temp_state_manager.read_state()
    assert result == {}

def test_read_state_corrupt_json(temp_state_manager):
    # Write invalid JSON directly to the file
    with open(temp_state_manager.state_file, 'w') as f:
        f.write("{ invalid json : [ ")
        
    # Read should swallow JSONDecodeError and return empty dict
    result = temp_state_manager.read_state()
    assert result == {}
