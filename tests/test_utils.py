import os
import sys
import hashlib
import tempfile
import pytest

# Add src to path so we can import modules
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from utils import calculate_checksum

def test_calculate_checksum_simple():
    """Test SHA1 calculation for a known string."""
    content = b"Hello Immich!"
    expected_sha1 = hashlib.sha1(content).hexdigest()
    
    with tempfile.NamedTemporaryFile(delete=False) as tmp:
        tmp.write(content)
        tmp_path = tmp.name
        
    try:
        assert calculate_checksum(tmp_path) == expected_sha1
    finally:
        os.remove(tmp_path)

def test_calculate_checksum_empty_file():
    """Test SHA1 for an empty file."""
    expected_sha1 = hashlib.sha1(b"").hexdigest() # da39a3ee5e6b4b0d3255bfef95601890afd80709
    
    with tempfile.NamedTemporaryFile(delete=False) as tmp:
        tmp_path = tmp.name
        
    try:
        assert calculate_checksum(tmp_path) == expected_sha1
    finally:
        os.remove(tmp_path)

def test_calculate_checksum_large_file():
    """Test SHA1 for a larger file (>64kb chunk size)."""
    # Create ~70kb file
    content = b"a" * 70000
    expected_sha1 = hashlib.sha1(content).hexdigest()
    
    with tempfile.NamedTemporaryFile(delete=False) as tmp:
        tmp.write(content)
        tmp_path = tmp.name
        
    try:
        assert calculate_checksum(tmp_path) == expected_sha1
    finally:
        os.remove(tmp_path)

def test_calculate_checksum_missing_file():
    """Test behavior when file doesn't exist."""
    assert calculate_checksum("/path/to/nonexistent/file") is None
