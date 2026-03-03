import os
import sys
import pytest
from unittest.mock import MagicMock, patch

# Add src to path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from monitor import ImmichEventHandler, ALLOWED_EXTENSIONS

class MockEvent:
    def __init__(self, src_path, is_directory=False):
        self.src_path = src_path
        self.is_directory = is_directory

@pytest.fixture
def event_handler():
    queue_manager_mock = MagicMock()
    return ImmichEventHandler(queue_manager_mock)

def test_on_created_allowed_extension(event_handler):
    """Test that a valid file extension is processed."""
    event = MockEvent("/tmp/photo.jpg")
    
    # Mock wait_for_file_completion and calculate_checksum to avoid disk IO/time delay
    with patch.object(event_handler, 'wait_for_file_completion', return_value=True), \
         patch('monitor.calculate_checksum', return_value='da39a3ee5e6b4b0d3255bfef95601890afd80709'):
        
        event_handler.on_created(event)
        
        # Should call add_to_queue
        event_handler.queue_manager.add_to_queue.assert_called_once()
        args, _ = event_handler.queue_manager.add_to_queue.call_args
        assert args[0]['path'] == "/tmp/photo.jpg"
        assert args[0]['checksum'] == 'da39a3ee5e6b4b0d3255bfef95601890afd80709'

def test_on_created_ignored_extension(event_handler):
    """Test that an invalid file extension (e.g. .xmp) is ignored."""
    event = MockEvent("/tmp/meta.xmp")
    
    with patch.object(event_handler, 'wait_for_file_completion', return_value=True):
        event_handler.on_created(event)
        
        # Should NOT call add_to_queue
        event_handler.queue_manager.add_to_queue.assert_not_called()

def test_on_created_directory(event_handler):
    """Test that directory creation events are ignored."""
    event = MockEvent("/tmp/NewFolder", is_directory=True)
    
    event_handler.on_created(event)
    event_handler.queue_manager.add_to_queue.assert_not_called()

def test_on_created_checksum_failure(event_handler):
    """Test behavior when checksum calculation fails (e.g. locked file)."""
    event = MockEvent("/tmp/photo.png")
    
    with patch.object(event_handler, 'wait_for_file_completion', return_value=True), \
         patch('monitor.calculate_checksum', return_value=None):
        
        event_handler.on_created(event)
        event_handler.queue_manager.add_to_queue.assert_not_called()

def test_wait_for_file_completion_success(event_handler):
    """Test file completion check (mocked filesystem calls)."""
    with patch('time.sleep'), \
         patch('os.path.getsize', side_effect=[100, 100]): # Stable size
        
        result = event_handler.wait_for_file_completion("/tmp/test.jpg", timeout=1, check_interval=0.1)
        assert result is True

def test_wait_for_file_completion_timeout(event_handler):
    """Test file completion timeout (size keeps changing)."""
    with patch('time.sleep'), \
         patch('time.time', side_effect=[0, 0.5, 1.1]), \
         patch('os.path.getsize', side_effect=[100, 105, 110]):
        
        result = event_handler.wait_for_file_completion("/tmp/test.jpg", timeout=1, check_interval=0.1)
        assert result is False
