import os
import sys
import pytest
from unittest.mock import patch, MagicMock

# Add src to path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from notifications import NotificationManager

@pytest.fixture
def notify_manager():
    return NotificationManager(app_name="Test App")

def test_send_basic(notify_manager):
    with patch('subprocess.Popen') as mock_popen:
        notify_manager.send("Hello", "World")
        
        mock_popen.assert_called_once()
        cmd = mock_popen.call_args[0][0]
        assert cmd[0] == "notify-send"
        assert "--app-name" in cmd
        assert "Test App" in cmd
        assert "Hello" in cmd
        assert "World" in cmd
        assert "-h" in cmd
        assert "string:x-canonical-private-synchronous:immich-sync-progress" in cmd

def test_send_with_progress_and_timeout(notify_manager):
    with patch('subprocess.Popen') as mock_popen:
        notify_manager.send("Uploading", "File.jpg", progress=50, timeout=2000)
        
        mock_popen.assert_called_once()
        cmd = mock_popen.call_args[0][0]
        
        # Check progress hint
        assert "int:value:50" in cmd
        
        # Check timeout
        assert "-t" in cmd
        assert "2000" in cmd

def test_send_file_not_found(notify_manager, caplog):
    # Simulate notify-send not being installed
    with patch('subprocess.Popen', side_effect=FileNotFoundError()):
        # Should not raise an exception
        notify_manager.send("Title", "Message")
        
        # It's currently expected to pass quietly according to source
        
def test_send_exception(notify_manager, caplog):
    # Simulate a generic error
    with patch('subprocess.Popen', side_effect=Exception("Test Error")):
        # Should catch and log error
        notify_manager.send("Title", "Message")
        
        assert "Failed to send notification: Test Error" in caplog.text
