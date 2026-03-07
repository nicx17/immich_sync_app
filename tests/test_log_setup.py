import os
import sys
import logging
from unittest.mock import patch

# Add src to path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

import log_setup

def test_setup_logging(tmp_path):
    with patch('log_setup.CONFIG_DIR', str(tmp_path)):
        log_setup.setup_logging()
        
        # Check if log file exists
        assert os.path.exists(str(tmp_path / "app.log"))
        
        # Check root logger has handlers attached
        root_logger = logging.getLogger()
        assert len(root_logger.handlers) >= 2
        
        # Test exception hook
        with patch('logging.Logger.critical') as mock_critical:
            try:
                raise ValueError("Test error")
            except ValueError:
                exc_type, exc_value, exc_traceback = sys.exc_info()
                sys.excepthook(exc_type, exc_value, exc_traceback)
                mock_critical.assert_called_with("Uncaught exception", exc_info=(exc_type, exc_value, exc_traceback))

def test_setup_logging_keyboard_interrupt():
    with patch('sys.__excepthook__') as mock_sys_hook:
        try:
            raise KeyboardInterrupt()
        except KeyboardInterrupt:
            exc_type, exc_value, exc_traceback = sys.exc_info()
            sys.excepthook(exc_type, exc_value, exc_traceback)
            mock_sys_hook.assert_called_with(exc_type, exc_value, exc_traceback)


def test_setup_logging_directory_creation_fails(tmp_path):
    # Make dir unwriteable or mock os.makedirs to raise an exception
    with patch('log_setup.CONFIG_DIR', str(tmp_path / "new_dir")):
        with patch('os.makedirs', side_effect=Exception("Failed to create dir")):
            # It should gracefully pass the exception inside the try/except block
            log_setup.setup_logging()

def test_setup_logging_file_handler_fails(tmp_path):
    with patch('log_setup.CONFIG_DIR', str(tmp_path)):
        with patch('logging.handlers.RotatingFileHandler', side_effect=Exception("Failed to wrap file")):
            # Should print warning to stderr and continue
            log_setup.setup_logging()

