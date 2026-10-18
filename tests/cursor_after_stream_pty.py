"""Run the actual async reader and DSR exchange through a Unix controlling PTY."""

import errno
import os
import pty
import select
import signal
import sys
import time


def run_query_fixture(binary, *, query, response, expected_queries, marker, timeout_seconds=30):
    pid, master = pty.fork()
    if pid == 0:
        os.execv(binary, [binary])
    pending = b""
    output = bytearray()
    queries = 0
    reaped = False
    status = None
    try:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            readable, _, _ = select.select([master], [], [], 0.1)
            if readable:
                try:
                    chunk = os.read(master, 4096)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break
                if not chunk:
                    break
                output.extend(chunk)
                pending += chunk
                while query in pending:
                    _, pending = pending.split(query, 1)
                    # A terminal reply arrives after the stream-drop wakeup.
                    time.sleep(0.08)
                    os.write(master, response)
                    queries += 1
            if not reaped:
                child, child_status = os.waitpid(pid, os.WNOHANG)
                if child:
                    reaped = True
                    status = child_status
        else:
            raise AssertionError("terminal query fixture timed out")
        if not reaped:
            _, status = os.waitpid(pid, 0)
            reaped = True
        assert os.waitstatus_to_exitcode(status) == 0, (
            f"child exit {os.waitstatus_to_exitcode(status)}, queries={queries}: "
            + output.decode(errors="replace")
        )
        assert queries == expected_queries, queries
        assert marker in output, output.decode(errors="replace")
        print(f"PASS: {queries} real PTY queries, including delayed replies during reader shutdown")
    finally:
        if not reaped:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(master)


if __name__ == "__main__":
    run_query_fixture(
        sys.argv[1],
        query=b"\x1b[6n",
        response=b"\x1b[10;18R",
        expected_queries=120,
        marker=b"CURSOR_HANDOFF_PASS",
    )
