import os
import sys
import pytest
from unittest.mock import patch, MagicMock

# Add src to path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from tray_icon import TrayIcon

@pytest.fixture
def mock_monitor():
    return MagicMock()

@pytest.fixture
def run_tray_headless(mock_monitor):
    # Mock pystray.Icon so we don't accidentally launch a real system tray icon which blocks tests
    with patch('pystray.Icon') as mock_icon_class:
        tray = TrayIcon(mock_monitor)
        
        yield tray, mock_icon_class.return_value

def test_tray_icon_init(run_tray_headless):
    tray, mock_icon_instance = run_tray_headless
    assert tray.monitor is not None
    assert tray.icon == mock_icon_instance

@patch('subprocess.Popen')
def test_tray_show_settings(mock_popen, run_tray_headless):
    tray, _ = run_tray_headless
    # Simulate clicking settings item
    tray.show_settings(None, None)
    
    # Should open settings_main.py in a subprocess
    mock_popen.assert_called_once()
    called_args = mock_popen.call_args[0][0]
    assert "settings_main.py" in called_args[1]

def test_tray_run(run_tray_headless):
    tray, mock_icon_instance = run_tray_headless
    
    tray.run()
    # verify pystray.Icon.run() was called
    mock_icon_instance.run.assert_called_once()

def test_tray_stop(run_tray_headless):
    tray, mock_icon_instance = run_tray_headless
    
    tray.stop()
    mock_icon_instance.stop.assert_called_once()
    tray.monitor.stop.assert_called_once()

@patch('sys.exit')
def test_tray_quit_app(mock_exit, run_tray_headless):
    tray, mock_icon_instance = run_tray_headless
    
    # Spy on stop method
    with patch.object(tray, 'stop') as mock_stop:
        tray.quit_app(None, None)
        
        mock_stop.assert_called_once()
        mock_exit.assert_called_once_with(0)
