"""Running commands in a PTY and driving them with keys."""

import errno
import fcntl
import os
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time

NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
SIZE_RE = re.compile(r"^(\d+)x(\d+)$")

KEYS = {
    "enter": b"\r", "return": b"\r", "tab": b"\t", "esc": b"\x1b", "escape": b"\x1b",
    "space": b" ", "backspace": b"\x7f", "bs": b"\x7f",
    "up": b"\x1b[A", "down": b"\x1b[B", "right": b"\x1b[C", "left": b"\x1b[D",
    "home": b"\x1b[H", "end": b"\x1b[F", "insert": b"\x1b[2~", "delete": b"\x1b[3~",
    "del": b"\x1b[3~", "pgup": b"\x1b[5~", "pageup": b"\x1b[5~", "pgdn": b"\x1b[6~",
    "pagedown": b"\x1b[6~", "shift-tab": b"\x1b[Z",
    "f1": b"\x1bOP", "f2": b"\x1bOQ", "f3": b"\x1bOR", "f4": b"\x1bOS",
    "f5": b"\x1b[15~", "f6": b"\x1b[17~", "f7": b"\x1b[18~", "f8": b"\x1b[19~",
    "f9": b"\x1b[20~", "f10": b"\x1b[21~", "f11": b"\x1b[23~", "f12": b"\x1b[24~",
}


class SpecError(ValueError):
    pass


def key_bytes(name):
    low = name.lower()
    if low in KEYS:
        return KEYS[low]
    m = re.fullmatch(r"(ctrl|alt)-(.)", low)
    if m and m[1] == "ctrl" and ("a" <= m[2] <= "z" or m[2] in "@[\\]^_"):
        return bytes([ord(m[2].upper()) & 0x1f])
    if m and m[1] == "alt":
        return b"\x1b" + name[-1].encode()
    if len(name) == 1:
        return name.encode()
    raise SpecError(f"unknown key {name!r}")


def parse_size(text, default):
    if not text:
        return default
    m = SIZE_RE.match(text)
    if not m or not (0 < int(m[1]) <= 500 and 0 < int(m[2]) <= 200):
        raise SpecError(f"size must be COLSxROWS up to 500x200, got {text!r}")
    return int(m[1]), int(m[2])


def parse_name(text, default_size):
    """`name` or `name@COLSxROWS`."""
    name, _, size = text.partition("@")
    name = name.strip()
    if not NAME_RE.match(name):
        raise SpecError(f"bad name {name!r}: use letters, digits, '.', '_' and '-'")
    return name, parse_size(size.strip(), default_size)


STEPS = ("wait", "wait-for", "type", "key", "snap")


def parse_shots(text, default_size):
    """The `shots` input: `name[@CxR]: command`, each optionally followed by indented
    steps that drive it (wait, wait-for, type, key, snap)."""
    shots = []
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        try:
            if raw[:1] in " \t":
                if not shots:
                    raise SpecError("a step needs a shot above it")
                verb, _, arg = line.partition(" ")
                if verb not in STEPS:
                    raise SpecError(f"unknown step {verb!r} (one of {', '.join(STEPS)})")
                if verb == "wait":
                    float(arg)
                elif verb == "key":
                    for k in arg.split() or [""]:
                        key_bytes(k)
                elif verb == "snap" and not NAME_RE.match(arg):
                    raise SpecError(f"bad snap name {arg!r}")
                elif verb in ("wait-for", "type") and not arg:
                    raise SpecError(f"{verb} needs text")
                shots[-1]["steps"].append((verb, arg))
                continue
            head, sep, command = line.partition(":")
            if not sep or not command.strip():
                raise SpecError("expected 'name: command'")
            name, size = parse_name(head, default_size)
            shots.append({"name": name, "size": size, "command": command.strip(), "steps": []})
        except (SpecError, ValueError) as e:
            raise SpecError(f"shots line {n}: {e}") from None
    return shots


class Session:
    """A command under bash in a PTY of cols x rows, recording what it prints."""

    def __init__(self, command, cols, rows):
        self.cols, self.rows = cols, rows
        self.master, slave = os.openpty()
        # Size the terminal before the program starts, so its first query sees it.
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        env = dict(os.environ, TERM="xterm-256color", COLUMNS=str(cols), LINES=str(rows))

        def child_setup():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self.proc = subprocess.Popen(["bash", "-c", command], stdin=slave, stdout=slave,
                                     stderr=slave, env=env, preexec_fn=child_setup,
                                     close_fds=True)
        os.close(slave)
        self.data = bytearray()
        self.eof = False

    def pump(self, seconds):
        """Read output for `seconds`, or until the program has gone."""
        end = time.monotonic() + seconds
        while not self.eof:
            left = end - time.monotonic()
            if left <= 0:
                return
            ready, _, _ = select.select([self.master], [], [], min(left, 0.05))
            if not ready:
                # The program may have exited while a grandchild keeps the PTY open.
                if self.proc.poll() is not None:
                    self.eof = True
                continue
            try:
                chunk = os.read(self.master, 65536)
            except OSError as e:
                if e.errno != errno.EIO:  # EIO on Linux: every slave fd is closed
                    raise
                chunk = b""
            if not chunk:  # EOF on macOS
                self.eof = True
            self.data += chunk

    def send(self, data):
        if not self.eof:
            try:
                os.write(self.master, data)
            except OSError:
                self.eof = True

    def close(self):
        """Stop recording, then end the program. Anything it prints on the way out
        (leaving the alternate screen, clearing) is not recorded."""
        if self.eof:
            try:
                self.proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                pass
        if self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                self.proc.kill()
        code = self.proc.wait()
        os.close(self.master)
        return code


def screen_text(termshot, data, cols, rows):
    with tempfile.NamedTemporaryFile(suffix=".pty") as f:
        f.write(data)
        f.flush()
        r = subprocess.run([termshot, "--size", f"{cols}x{rows}", "--text", "-", f.name],
                           capture_output=True)
    return r.stdout.decode("utf-8", "replace")


def run(shot, timeout, termshot, settle=0.3):
    """Run a shot and its steps. Returns a list of screens, each a dict with `name`,
    `log` (bytes), and `note` (why the screen was taken), and the exit status of the
    program, or None when it was still running."""
    cols, rows = shot["size"]
    s = Session(shot["command"], cols, rows)
    deadline = time.monotonic() + timeout
    screens, notes = [], []
    if not shot["steps"]:
        s.pump(timeout)
    for verb, arg in shot["steps"]:
        if s.eof or time.monotonic() >= deadline:
            notes.append(f"stopped before `{verb} {arg}`")
            break
        if verb == "wait":
            s.pump(min(float(arg), deadline - time.monotonic()))
        elif verb == "wait-for":
            while arg not in screen_text(termshot, bytes(s.data), cols, rows):
                if s.eof or time.monotonic() >= deadline:
                    notes.append(f"`{arg}` never appeared")
                    break
                s.pump(0.1)
        elif verb == "type":
            s.send(arg.encode())
            s.pump(0.05)
        elif verb == "key":
            for k in arg.split():
                s.send(key_bytes(k))
                s.pump(0.05)
        elif verb == "snap":
            s.pump(settle)
            screens.append({"name": f"{shot['name']}.{arg}", "log": bytes(s.data), "note": ""})
    if shot["steps"] and not s.eof:
        s.pump(settle)
    running = not s.eof
    code = s.close()
    exit_code = None if running else code
    note = "; ".join(notes)
    if running and not shot["steps"]:
        note = note or f"still running at {timeout:g}s"
    screens.append({"name": shot["name"], "log": bytes(s.data), "note": note})
    return screens, exit_code
