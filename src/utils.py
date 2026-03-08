import hashlib
import logging
logger = logging.getLogger(__name__)
import os

def calculate_checksum(file_path):
    """
    Calculate the SHA-1 checksum of a file.
    Immich uses SHA-1 for deduplication checks.
    """
    logger.debug(f"Calculating SHA-1 for: {file_path}")
    if not os.path.exists(file_path):
        logger.error(f"File not found for checksum: {file_path}")
        return None

    sha1 = hashlib.sha1()
    
    # Read in chunks to handle large files (e.g. 4K video) without memory issues
    BUFFER_SIZE = 65536  # 64kb
    
    try:
        with open(file_path, 'rb') as f:
            while True:
                data = f.read(BUFFER_SIZE)
                if not data:
                    break
                sha1.update(data)
        checksum = sha1.hexdigest()
        logger.debug(f"Checksum ({file_path}): {checksum}")
        return checksum
    except IOError as e:
        # File might be locked or unreadable
        logger.error(f"IOError calculating checksum for {file_path}: {e}")
        return None
