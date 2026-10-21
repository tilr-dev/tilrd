"""Hold a process-scoped guard while the shared build owns its directory lock."""
import fcntl
import os
import sys

prefix, script = sys.argv[1:]
# Keep this inode: unlinking it would let contenders lock different files.
guard = os.open(prefix + ".guard", os.O_CREAT | os.O_RDWR, 0o600)
try:
    fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
except BlockingIOError:
    print(f"Build already in progress: {prefix}.lock; rerun after it finishes.",
          file=sys.stderr)
    sys.exit(1)
os.set_inheritable(guard, True)
os.environ["TILR_GHOSTTY_VT_LOCKED_PREFIX"] = prefix
os.execv(script, [script])
