#!/usr/bin/env python3
"""Command line helper for the bevy_input_driver BRP methods (Python 3 standard library only)."""

import argparse
import json
import sys
import time
import urllib.error
import urllib.request

EXIT_TIMEOUT = 1
EXIT_UNREACHABLE = 2
EXIT_BRP_ERROR = 3
POLL_SECONDS = 0.1


class DriverUnreachable(Exception):
    """Raised when the game does not answer on the chosen port."""


def call(port, method, params=None):
    """Sends one JSON-RPC 2.0 request and returns the `result`, or exits 3 on a BRP error."""
    body = {"jsonrpc": "2.0", "id": 1, "method": method}
    if params is not None:
        body["params"] = params
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}/",
        data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            reply = json.load(response)
    except (urllib.error.URLError, ConnectionError, TimeoutError) as problem:
        raise DriverUnreachable(
            f"cannot reach the game on 127.0.0.1:{port} ({problem}). "
            "Is it running with the input driver enabled?"
        ) from problem
    if "error" in reply:
        print(json.dumps(reply["error"], indent=2), file=sys.stderr)
        sys.exit(EXIT_BRP_ERROR)
    return reply.get("result")


def read_commands_from_file(path):
    """Reads a JSON Lines file, skipping blank lines and lines starting with // or #."""
    commands = []
    with open(path, encoding="utf-8") as handle:
        for number, raw_line in enumerate(handle, start=1):
            line = raw_line.strip()
            if not line or line.startswith("//") or line.startswith("#"):
                continue
            try:
                commands.append(json.loads(line))
            except json.JSONDecodeError as problem:
                print(f"{path} line {number}: {problem}", file=sys.stderr)
                sys.exit(EXIT_TIMEOUT)
    return commands


def wait_until_idle(port, timeout, game_was_reachable=False):
    """Polls drive/status until idle. Returns the final status, or None on timeout.

    Once the game has answered at least once (here or in the call that sent the commands), a
    later loss of the connection means it exited, for example after a quit command. That prints
    {"game_exited": true} and exits 0.
    """
    deadline = time.monotonic() + timeout
    while True:
        try:
            status = call(port, "drive/status")
        except DriverUnreachable:
            if not game_was_reachable:
                raise
            print(json.dumps({"game_exited": True}))
            sys.exit(0)
        game_was_reachable = True
        if status.get("idle"):
            return status
        if time.monotonic() >= deadline:
            print(json.dumps(status, indent=2))
            print(f"timed out after {timeout} seconds before the driver went idle", file=sys.stderr)
            return None
        time.sleep(POLL_SECONDS)


def send_commands(args, commands):
    """Sends commands as one array, then optionally waits."""
    print(json.dumps(call(args.port, "drive/send", commands), indent=2))
    if args.wait:
        finish_wait(args, game_was_reachable=True)


def finish_wait(args, game_was_reachable=False):
    """Waits for idle and exits 1 on timeout."""
    status = wait_until_idle(args.port, args.timeout, game_was_reachable)
    if status is None:
        sys.exit(EXIT_TIMEOUT)
    print(json.dumps(status, indent=2))


def build_parser():
    """Builds the argument parser."""
    parser = argparse.ArgumentParser(
        description="Drive a Bevy game running bevy_input_driver.",
        epilog=(
            "exit codes: 0 success (also when the game exited while waiting, after printing "
            '{"game_exited": true}), 1 timeout or bad input, 2 game unreachable at start, '
            "3 error reply from the game"
        ),
    )
    parser.add_argument("--port", type=int, default=15702)
    commands = parser.add_subparsers(dest="action", required=True)

    send = commands.add_parser("send", help="send JSON commands")
    send.add_argument("commands", nargs="+", help="a JSON command object or array")
    send.add_argument("--wait", action="store_true", help="wait until idle afterward")
    send.add_argument("--timeout", type=float, default=30.0)

    send_file = commands.add_parser("send-file", help="send a JSON Lines file")
    send_file.add_argument("path")
    send_file.add_argument("--wait", action="store_true", help="wait until idle afterward")
    send_file.add_argument("--timeout", type=float, default=30.0)

    commands.add_parser("status", help="print driver status")
    commands.add_parser("clear", help="empty the queue and release held input")

    wait = commands.add_parser("wait", help="wait until the driver is idle")
    wait.add_argument("--timeout", type=float, default=30.0)
    return parser


def collect_send_commands(raw_arguments):
    """Flattens each argument, an object or an array, into one command list."""
    commands = []
    for raw in raw_arguments:
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError as problem:
            print(f"invalid JSON argument {raw!r}: {problem}", file=sys.stderr)
            sys.exit(EXIT_TIMEOUT)
        commands.extend(parsed if isinstance(parsed, list) else [parsed])
    return commands


def run(args):
    """Runs the chosen action."""
    if args.action == "send":
        send_commands(args, collect_send_commands(args.commands))
    elif args.action == "send-file":
        send_commands(args, read_commands_from_file(args.path))
    elif args.action == "status":
        print(json.dumps(call(args.port, "drive/status"), indent=2))
    elif args.action == "clear":
        print(json.dumps(call(args.port, "drive/clear"), indent=2))
    elif args.action == "wait":
        finish_wait(args)


def main():
    """Entry point."""
    args = build_parser().parse_args()
    try:
        run(args)
    except DriverUnreachable as problem:
        print(str(problem), file=sys.stderr)
        sys.exit(EXIT_UNREACHABLE)


if __name__ == "__main__":
    main()
