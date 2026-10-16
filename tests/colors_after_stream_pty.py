"""Exercise the actual palette query with the shared controlling-PTY driver."""

import sys


from cursor_after_stream_pty import run_query_fixture


if __name__ == "__main__":
    run_query_fixture(
        sys.argv[1],
        query=b"\x1b]11;?\x1b\\",
        response=b"\x1b]10;rgb:0000/0000/0000\x07\x1b]11;rgb:ffff/ffff/ffff\x07",
        expected_queries=12,
        marker=b"PALETTE_HANDOFF_PASS",
        timeout_seconds=15,
    )
