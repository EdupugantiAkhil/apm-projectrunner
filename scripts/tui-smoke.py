#!/usr/bin/env python3
"""AppCUI PTY smoke suite.

Dependency: Python 3 plus ``pyte`` (install with ``python3 -m pip install pyte``).
If pyte is unavailable the suite prints a skip message and exits successfully.
"""

from __future__ import annotations

import codecs
import fcntl
import os
from pathlib import Path
import pty
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time

try:
    import pyte
except ImportError:
    print("SKIP: scripts/tui-smoke.py requires pyte (python3 -m pip install pyte)")
    raise SystemExit(0)


ROWS = 35
COLS = 120
ESC = b"\x1b"
ENTER = b"\r"
TAB = b"\t"
F1 = b"\x1bOP"
F2 = b"\x1bOQ"
F7 = b"\x1b[18~"
CTRL_Q = b"\x11"


class SmokeFailure(RuntimeError):
    pass


class Terminal:
    def __init__(self, argv: list[str], cwd: Path, env: dict[str, str]):
        master, slave = pty.openpty()
        # AppCUI deliberately waits for a usable terminal. Set the dimensions
        # before the child can inspect the slave; a new PTY otherwise starts 0x0.
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.master = master
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.process = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            close_fds=True,
            start_new_session=True,
        )
        os.close(slave)

    def text(self) -> str:
        return "\n".join(self.screen.display)

    def send(self, data: bytes) -> None:
        os.write(self.master, data)
        self.pump(0.08)

    def type(self, value: str) -> None:
        self.send(value.encode())

    def pump(self, duration: float = 0.1) -> None:
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            timeout = max(0.0, min(0.05, deadline - time.monotonic()))
            ready, _, _ = select.select([self.master], [], [], timeout)
            if not ready:
                continue
            try:
                data = os.read(self.master, 65536)
            except OSError:
                return
            if not data:
                return
            decoded = self.decoder.decode(data)
            if decoded:
                self.stream.feed(decoded)

    def wait_for(self, needle: str, timeout: float = 10.0) -> str:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump(0.1)
            screen = self.text()
            if needle in screen:
                return screen
            if self.process.poll() is not None:
                raise SmokeFailure(
                    f"TUI exited with {self.process.returncode} while waiting for {needle!r}\n{screen}"
                )
        raise SmokeFailure(f"timed out waiting for {needle!r}\n{self.text()}")

    def wait_exit(self, timeout: float = 10.0) -> int:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump(0.1)
            status = self.process.poll()
            if status is not None:
                return status
        raise SmokeFailure(f"TUI did not exit within {timeout:.0f}s\n{self.text()}")

    def close(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=2)
        os.close(self.master)


class Results:
    def __init__(self) -> None:
        self.number = 0
        self.failures = 0

    def check(self, label: str, action) -> None:
        self.number += 1
        try:
            action()
        except Exception as error:  # each assertion must report independently
            self.failures += 1
            print(f"{self.number}. FAIL — {label}: {error}")
        else:
            print(f"{self.number}. PASS — {label}")


def run_checked(argv: list[str], cwd: Path | None = None) -> None:
    subprocess.run(argv, cwd=cwd, check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)


def resolve_binary(root: Path) -> Path:
    configured = os.environ.get("SWITCHYARD_BIN")
    if configured:
        binary = Path(configured).expanduser().resolve()
        if not binary.is_file():
            raise SmokeFailure(f"SWITCHYARD_BIN does not name a file: {binary}")
        return binary
    run_checked(["cargo", "build", "-p", "switchyard-cli", "--offline"], root)
    binary = root / "target" / "debug" / "switchyard"
    if not binary.is_file():
        raise SmokeFailure(f"cargo build did not create {binary}")
    return binary


def alt(letter: str) -> bytes:
    return ESC + letter.lower().encode("ascii")


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    results = Results()
    terminal: Terminal | None = None
    workspace = Path(tempfile.mkdtemp(prefix="switchyard-tui-smoke-"))
    try:
        binary = resolve_binary(root)
        project = workspace / "project"
        run_checked([str(binary), "init", str(project), "--name", "tui-smoke"])

        repository = project / "local-repo"
        repository.mkdir()
        (repository / "README.md").write_text("local PTY smoke repository\n", encoding="utf-8")
        run_checked(["git", "init", "-q"], repository)
        run_checked(["git", "add", "README.md"], repository)
        run_checked(
            [
                "git",
                "-c",
                "user.name=Switchyard Smoke",
                "-c",
                "user.email=smoke@switchyard.invalid",
                "commit",
                "-qm",
                "initial",
            ],
            repository,
        )

        env = os.environ.copy()
        env["SWITCHYARD_BIN"] = str(binary)
        env.setdefault("TERM", "xterm-256color")
        terminal = Terminal([str(binary), "tui", str(project)], project, env)

        terminal.wait_for("Switchyard", 15)
        tabs = [
            ("h", "First-run checklist", "Home"),
            ("c", "Repositories and checkouts", "Code"),
            ("p", "Startup profile library", "Profiles"),
            ("i", "Instances", "Instances"),
            ("n", "Route matrix", "Connections"),
            ("d", "Execution devices", "Devices"),
            ("o", "Project run actions", "Operations"),
        ]
        for key, title, name in tabs:
            results.check(
                f"Alt+{key.upper()} opens {name}",
                lambda key=key, title=title: (
                    terminal.send(alt(key)),
                    terminal.wait_for(title),
                ),
            )

        def help_round_trip() -> None:
            terminal.send(F1)
            terminal.wait_for("Switchyard help")
            terminal.send(ESC)
            terminal.wait_for("Project run actions")

        results.check("F1 opens help and Escape closes it", help_round_trip)

        def cancel_add() -> None:
            terminal.send(alt("c"))
            terminal.wait_for("Repositories and checkouts")
            terminal.send(F2)
            terminal.wait_for("Add code")
            terminal.send(ESC)
            terminal.wait_for("Repositories and checkouts")

        results.check("F2 opens Add code and Escape cancels", cancel_add)

        def register_local() -> None:
            terminal.send(F2)
            terminal.wait_for("Add code")
            terminal.send(TAB)  # Mode -> Directory
            terminal.type(str(repository))
            terminal.send(ENTER)
            terminal.wait_for("Source registered successfully.", 20)
            terminal.wait_for("local-repo")

        results.check("local Git repository registers through background state job", register_local)

        def clone_local() -> None:
            terminal.send(F2)
            terminal.wait_for("Add code")
            # The Mode DropDownList must be opened before arrows move the
            # highlight: Space opens, Down x2 highlights Clone, Enter selects.
            terminal.send(b" ")
            terminal.pump(0.3)
            terminal.send(b"\x1b[B")
            terminal.send(b"\x1b[B")
            terminal.send(ENTER)
            terminal.wait_for("Clone address")
            terminal.send(TAB)
            terminal.type(str(repository))
            terminal.send(TAB)
            terminal.type("cloned-repo")
            terminal.send(ENTER)
            terminal.wait_for("Source cloned and registered successfully.", 45)
            terminal.wait_for("Repositories and checkouts")
            terminal.wait_for("cloned-repo")

        results.check("clone handoff re-execs onto Code with success notice", clone_local)

        def template_instance() -> None:
            # let any post-clone refresh finish before driving new operations
            terminal.pump(2.0)
            terminal.send(alt("i"))
            terminal.wait_for("Instances")
            terminal.wait_for("web")

        results.check("Instances shows the initialized template instance", template_instance)

        def validate() -> None:
            # Enter accepts the preview deterministically (Alt hotkeys can be
            # split across pty reads). The post-restart refresh may still hold
            # the single-operation gate; retry until the validate runs.
            for _ in range(4):
                terminal.send(F7)
                terminal.wait_for("Validate deployment")
                terminal.send(ENTER)
                terminal.pump(1.5)
                if "already running" in terminal.text():
                    terminal.pump(3.0)
                    continue
                terminal.wait_for("Finished(0)", 45)
                return
            raise SmokeFailure("validate remained blocked by the operation gate")

        results.check("F7 preview accepts with Enter and finishes with Finished(0)", validate)

        def quit_tui() -> None:
            # Dismiss any dialog a failed earlier step may have left open, so
            # this assertion stays independent. Esc on the shell itself also
            # quits, so tolerate the app exiting at any point here.
            try:
                terminal.send(ESC)
                terminal.pump(0.4)
                if terminal.process.poll() is None:
                    terminal.send(CTRL_Q)
            except OSError:
                pass
            status = terminal.wait_exit(10)
            if status != 0:
                raise SmokeFailure(f"Ctrl+Q exited with status {status}")

        results.check("Ctrl+Q exits cleanly", quit_tui)
    except Exception as error:
        results.number += 1
        results.failures += 1
        print(f"{results.number}. FAIL — suite setup or driver: {error}")
    finally:
        if terminal is not None:
            terminal.close()
        shutil.rmtree(workspace, ignore_errors=True)

    print(f"RESULT: {results.number - results.failures} passed, {results.failures} failed")
    return 1 if results.failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
