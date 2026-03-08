import sys
import logging
import logging.handlers
import os

# We shouldn't import config directly to avoid circular dependency early,
# so we redefine CONFIG_DIR minimally or import it safely
from config import CONFIG_DIR

def setup_logging():
    if not os.path.exists(CONFIG_DIR):
        try:
            os.makedirs(CONFIG_DIR, exist_ok=True)
        except Exception:
            pass

    log_file = os.path.join(CONFIG_DIR, "app.log")
    
    # Determine level
    level = logging.DEBUG if os.environ.get("MIMICK_DEBUG") == "1" else logging.INFO
    
    # Configure root logger with better format
    log_format = '%(asctime)s [%(levelname)s] %(name)s: %(message)s'
    date_format = '%Y-%m-%d %H:%M:%S'
    
    handlers = [logging.StreamHandler(sys.stdout)]
    
    # Only attach file handler if we want logs to disk
    if os.environ.get("MIMICK_NO_LOG_FILE") != "1":
        try:
            handlers.append(logging.handlers.RotatingFileHandler(
                log_file, maxBytes=5*1024*1024, backupCount=2, encoding='utf-8'
            ))
        except Exception as e:
            # Fallback if permissions or something fails
            print(f"Warning: Could not setup file logging: {e}", file=sys.stderr)

    logging.basicConfig(
        level=level,
        format=log_format,
        datefmt=date_format,
        handlers=handlers,
        force=True
    )
    
    # Optional: Lower noise from common third-party libraries unless in strict debug mode
    if level != logging.DEBUG:
        logging.getLogger("urllib3").setLevel(logging.WARNING)
        logging.getLogger("watchdog").setLevel(logging.WARNING)
        logging.getLogger("PIL").setLevel(logging.WARNING)
    
    # Capture unhandled exceptions
    def handle_exception(exc_type, exc_value, exc_traceback):
        if issubclass(exc_type, KeyboardInterrupt):
            sys.__excepthook__(exc_type, exc_value, exc_traceback)
            return
        logging.getLogger("mimick.Core").critical("Uncaught exception", exc_info=(exc_type, exc_value, exc_traceback))
        
    sys.excepthook = handle_exception
