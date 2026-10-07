"""Comparing two screens, and writing the pull request comment."""

import difflib
import html
import unicodedata

SERVER_LINK = "https://github.com/momiji-rs/termshot"
MARK = "<!-- termshot-action:{id} -->"
LIMIT = 60000  # GitHub refuses comments over 65,536 characters

# ---------------------------------------------------------------- cell diff

DEFAULT_FG, DEFAULT_BG = "#dbe7f7", "#111823"
ATTRS = ("bold", "italic", "underline", "double_underline", "strike")
SGR = {"bold": "1", "italic": "3", "underline": "4", "double_underline": "21", "strike": "9"}
LIT_BG = "#8a1c4a"


def cells(screen):
    """{(row, col): (char, fg, bg, attrs)} from termshot --json; blank cells are left out."""
    grid = {}
    for row, runs in enumerate(screen["lines"]):
        for run in runs:
            col = run["col"]
            attrs = tuple(a for a in ATTRS if run.get(a))
            for ch in run["text"]:
                fg = run["fg"]
                if ch == " " and not set(attrs) & {"underline", "double_underline", "strike"}:
                    fg = None  # a plain space looks the same in any foreground
                if not (ch == " " and fg is None and run["bg"] == DEFAULT_BG):
                    grid[(row, col)] = (ch, fg, run["bg"], attrs)
                col += 2 if unicodedata.east_asian_width(ch) in "WF" else 1
    return grid


def rgb(hex_colour):
    return tuple(int(hex_colour[i:i + 2], 16) for i in (1, 3, 5))


def mix(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(rgb(a), rgb(b)))


def diff_log(head, base):
    """A PTY log of `head` with the cells that differ from `base` at full strength
    and the rest dimmed, and the set of changed cells. termshot renders it like any
    other log."""
    new, old = cells(head), cells(base)
    changed = {k for k in set(new) | set(old) if new.get(k) != old.get(k)}
    out = ["\x1b[?25l\x1b[2J"]
    for (row, col) in sorted(set(new) | changed):
        ch, fg, bg, attrs = new.get((row, col), (" ", None, DEFAULT_BG, ()))
        fg = fg or DEFAULT_FG
        if (row, col) in changed:
            # Keep a changed cell's own colours, so a colour change shows as itself;
            # only cells on the default background are lit.
            f, b = rgb(fg), rgb(LIT_BG if bg == DEFAULT_BG else bg)
        else:
            f, b = mix(fg, bg, 0.7), mix(bg, DEFAULT_BG, 0.75)
        sgr = ";".join(["0", *(SGR[a] for a in attrs), "38;2;%d;%d;%d" % f, "48;2;%d;%d;%d" % b])
        out.append(f"\x1b[{row + 1};{col + 1}H\x1b[{sgr}m{ch}")
    out.append("\x1b[0m")
    return "".join(out).encode(), changed


def describe(changed):
    rows = sorted({r for r, _ in changed})
    if not rows:
        return "no cells changed (an image in the screen did)"
    spans, start = [], rows[0]
    for a, b in zip(rows, rows[1:] + [None]):
        if b != a + 1:
            spans.append(f"{start + 1}" if start == a else f"{start + 1}–{a + 1}")
            start = b
    n = len(changed)
    return f"{n} cell{'s' * (n != 1)} changed, in row{'s' * (len(rows) != 1)} {', '.join(spans)}"


def text_diff(old, new):
    lines = difflib.unified_diff(old.splitlines(), new.splitlines(), lineterm="", n=1)
    return "\n".join(list(lines)[2:])  # the ---/+++ header says nothing here


# ---------------------------------------------------------------- markdown


def fence(text, lang="", cap=None):
    if cap and len(text) > cap:
        text = text[:cap] + "\n… (cut)"
    ticks = "```"
    while ticks in text:
        ticks += "`"
    return f"{ticks}{lang}\n{text}\n{ticks}"


def code(text):
    return f"<code>{html.escape(text)}</code>"


def img(url, alt, width=None):
    w = f' width="{width}"' if width else ""
    return f'<img src="{html.escape(url)}" alt="{html.escape(alt)}"{w}>'


def report(ctx, shots, base_names, url, compact=False):
    """The comment and job summary. `base_names` is the set of screens in the baseline,
    or None without one; `url(path)` is an object's image URL, or None when images
    aren't published."""
    by = lambda st: [s for s in shots if s["status"] == st]
    changed, new, same = by("changed"), by("new"), by("unchanged")
    removed = sorted(base_names - {s["name"] for s in shots}) if base_names is not None else []
    cap = 1500 if compact else 20000

    counts = [f"{len(shots)} screen{'s' * (len(shots) != 1)}"]
    if base_names is not None:
        counts += [f"{len(changed)} changed"] if changed else []
        counts += [f"{len(new)} new"] if new else []
        counts += [f"{len(removed)} removed"] if removed else []
        if not changed and not new and not removed:
            counts.append("no visual changes")
    out = [MARK.format(id=ctx["id"]), f"### 📸 termshot · {' · '.join(counts)}", ""]
    if base_names is None and ctx.get("base_ref"):
        out += [f"_No baseline for {code(ctx['base_ref'])} yet: it is stored when this "
                f"workflow runs on a push to it._", ""]

    def meta(s):
        bits = [code(s["label"])]
        if s.get("steps"):
            bits.append(f"{s['steps']} step{'s' * (s['steps'] != 1)}")
        if s.get("note"):
            bits.append(html.escape(s["note"]))
        if s.get("exit"):
            bits.append(f"⚠️ exit {s['exit']}")
        return " · ".join(bits)

    def picture(path, alt, width=None):
        u = url(path) if path else None
        return [img(u, alt, width), ""] if u else []

    for s in changed:
        out += [f"#### ✏️ {code(s['name'])} changed", meta(s), ""]
        bu, hu = url(s.get("base_png")), url(s.get("png_path"))
        if bu and hu:
            out += ["| before | after |", "|---|---|",
                    f"| {img(bu, s['name'] + ' before')} | {img(hu, s['name'] + ' after')} |", ""]
        if s.get("diff_desc"):
            out += [f"**diff** · {s['diff_desc']}", ""]
            out += picture(s.get("diff_path"), s["name"] + " diff")
        d = text_diff(s.get("base_text", ""), s["text"])
        if d:
            out += ["<details open><summary>text diff</summary>", "", fence(d, "diff", cap),
                    "", "</details>", ""]
        else:
            out += ["_Same text; colours or attributes changed._", ""]
    for s in new:
        out += [f"#### {'🆕' if base_names is not None else '🖥️'} {code(s['name'])}", meta(s), ""]
        out += picture(s.get("png_path"), s["name"])
        if not compact:
            out += ["<details><summary>text</summary>", "", fence(s["text"], cap=cap), "",
                    "</details>", ""]
    if same:
        out += [f"<details><summary>✅ {len(same)} unchanged: "
                + ", ".join(code(s["name"]) for s in same) + "</summary>", ""]
        for s in same:
            out += [f"**{code(s['name'])}** · {meta(s)}", ""]
            pic = picture(s.get("png_path"), s["name"], 480)
            out += pic or ([] if compact else [fence(s["text"], cap=cap), ""])
        out += ["</details>", ""]
    if removed:
        out += ["🗑️ removed: " + ", ".join(code(n) for n in removed), ""]
    foot = f"<sub>Rendered by [termshot]({SERVER_LINK}) {html.escape(ctx['version'])}"
    if ctx.get("sha"):
        foot += f" at {ctx['sha'][:7]}"
    if ctx.get("run_url"):
        foot += f" · [run]({ctx['run_url']})"
    out.append(foot + "</sub>")
    body = "\n".join(out)
    if len(body) > LIMIT and not compact:
        return report(ctx, shots, base_names, url, compact=True)
    if len(body) > LIMIT:
        body = body[:LIMIT] + "\n\n_The comment was cut at GitHub's size limit; the job " \
                              "summary and the artifact have every screen._"
    return body
