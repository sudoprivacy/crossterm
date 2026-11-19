"""Answer the first cursor query, leave the second unanswered and type a key."""

import errno
import os
import pty
import select
import signal
import sys
import time


def main(binary, timeout_seconds=30):
    pid, master = pty.fork()
    if pid == 0:
        os.execv(binary, [binary])
    output, pending, queries, status = bytearray(), b"", 0, None
    try:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline and status is None:
            readable, _, _ = select.select([master], [], [], 0.1)
            if readable:
                try:
                    chunk = os.read(master, 4096)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    chunk = b""
                output.extend(chunk)
                pending += chunk
                while b"\x1b[6n" in pending:
                    _, pending = pending.split(b"\x1b[6n", 1)
                    queries += 1
                    # Reports are 1-based: (17, 9) is row 10, column 18.
                    os.write(master, b"\x1b[10;18R" if queries == 1 else b"k")
            child, child_status = os.waitpid(pid, os.WNOHANG)
            if child:
                status = child_status
        assert status is not None, "fixture timed out: " + output.decode(errors="replace")
        assert os.waitstatus_to_exitcode(status) == 0, output.decode(errors="replace")
        assert queries == 2, queries
        assert b"CURSOR_WITHIN_PASS" in output, output.decode(errors="replace")
        print("PASS: answered query, bounded silent query, queued key preserved")
    finally:
        if status is None:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(master)


if __name__ == "__main__":
    main(sys.argv[1])
