"""Wait for a file a board or workspace agent writes, up to a deadline."""
import time


def wait_for(path, seconds):
    """True when `path` exists before `seconds` pass."""
    end = time.monotonic() + seconds
    while time.monotonic() < end and not path.exists():
        time.sleep(.1)
    return path.exists()
