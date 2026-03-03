import os
import sys
import pytest
import queue
import time
from unittest.mock import MagicMock, patch

# Add src to path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from queue_manager import QueueManager

@pytest.fixture
def queue_manager():
    """
    Fixture providing a QueueManager with mocked dependencies.
    Prevents real file I/O or API calls.
    """
    with patch('queue_manager.Config') as MockConfig, \
         patch('queue_manager.ImmichApiClient') as MockClient, \
         patch('queue_manager.NotificationManager') as MockNotifier, \
         patch('queue_manager.StateManager') as MockState:
        
        # Setup Mocks
        config_instance = MockConfig.return_value
        config_instance.get_api_key.return_value = "test-key"
        config_instance.internal_url = "http://internal"
        config_instance.external_url = "https://external"
        
        qm = QueueManager()
        # Ensure we don't actually spawn threads in __init__ (it doesn't, but start() does)
        return qm

def test_add_to_queue(queue_manager):
    """Test adding an item to the queue updates stats and state."""
    task = {'path': '/tmp/test.jpg', 'checksum': 'abc'}
    
    queue_manager.add_to_queue(task)
    
    # Assertions
    assert queue_manager.upload_queue.qsize() == 1
    assert queue_manager.total_queued_session == 1
    
    # Check if state was published
    queue_manager.state_manager.write_state.assert_called()
    args, _ = queue_manager.state_manager.write_state.call_args
    state = args[0]
    assert state['status'] == 'uploading'
    assert state['queue_size'] == 1

def test_process_queue_success(queue_manager):
    """Test successful processing of a queue item."""
    # Setup successful upload
    queue_manager.api_client.upload_asset.return_value = "asset-123"
    
    task = {'path': '/tmp/test.jpg', 'checksum': 'abc'}
    queue_manager.upload_queue.put(task)
    queue_manager.total_queued_session = 1 # Manually set for stats consistency
    
    # Run ONE iteration of _process_queue logic
    # We can't run the actual loop easily, so we extract logic or just run with timeout?
    # Better: Inspect internal handler logic OR verify the loop behavior with a thread.
    
    # Let's run the worker in a thread but stop it quickly
    queue_manager.start()
    
    # Wait for queue to empty (processed)
    time.sleep(0.5)
    
    queue_manager.stop()
    
    # Assertions
    assert queue_manager.upload_queue.empty()
    assert queue_manager.retry_queue.empty()
    assert queue_manager.processed_session == 1
    
    # verify API calls
    queue_manager.api_client.upload_asset.assert_called_with('/tmp/test.jpg', 'abc')
    # verify album creation/addition logic
    queue_manager.api_client.get_or_create_album.assert_called()

def test_process_queue_failure(queue_manager):
    """Test failed upload (should move to retry queue)."""
    # Setup FAILED upload
    queue_manager.api_client.upload_asset.return_value = None
    
    task = {'path': '/tmp/fail.jpg', 'checksum': 'bad'}
    queue_manager.upload_queue.put(task)
    
    queue_manager.start()
    time.sleep(0.5)
    queue_manager.stop()
    
    # Assertions
    assert queue_manager.upload_queue.empty()
    assert not queue_manager.retry_queue.empty() # Should be in retry queue
    assert queue_manager.retry_queue.qsize() == 1
    assert queue_manager.processed_session == 0 # Failed items don't count as processed yet

def test_handle_duplicate_asset(queue_manager):
    """Test that duplicate assets are handled gracefully (success)."""
    queue_manager.api_client.upload_asset.return_value = "DUPLICATE"
    
    task = {'path': '/tmp/dupe.jpg', 'checksum': 'dup'}
    queue_manager.add_to_queue(task)
    
    # Manually invoke the internal handler to test logic without threading
    success = queue_manager._handle_upload(task)
    
    assert success is True
    # Should NOT try to add duplicate to album if we don't have an ID?
    # Current implementation: if "DUPLICATE", returns True immediately without add_to_album
    queue_manager.api_client.add_assets_to_album.assert_not_called()
