"""Check that each <log>.json in a directory parses, has the shape --json
promises, and spells the rows of <log>.txt beside it. Run by test.sh.

    python3 tests/grids/check.py tests/grids
"""
import json
import pathlib
import sys
import unicodedata


def width(text):
    """The fewest and the most cells text can take. Python's Unicode version
    may be older than termshot's tables, so a code point it has unassigned
    could be either width; every other one is wide when it is W or F."""
    unknown = sum(unicodedata.category(ch) == "Cn" for ch in text)
    wide = sum(unicodedata.east_asian_width(ch) in "WF" for ch in text if unicodedata.category(ch) != "Cn")
    least = len(text) + wide
    return least, least + unknown


def check(json_path):
    grid = json.loads(json_path.read_text(encoding="utf-8"))
    rows = json_path.with_suffix(".txt").read_text(encoding="utf-8").split("\n")[:-1]
    assert len(grid["lines"]) == grid["rows"] == len(rows), "row count"
    cursor = grid["cursor"]
    if cursor is not None:
        assert 0 <= cursor["col"] < grid["cols"] and 0 <= cursor["row"] < grid["rows"], "cursor off the grid"
    for r, (runs, row) in enumerate(zip(grid["lines"], rows)):
        # Runs cover the row from column 0 without gaps, so each starts where
        # the one before ends, and the last ends inside the grid.
        least = most = 0
        for run in runs:
            assert least <= run["col"] <= most, f"row {r}: run at col {run['col']}, want {least}..{most}"
            assert run["text"], f"row {r}: empty run at col {run['col']}"
            low, high = width(run["text"])
            least, most = run["col"] + low, run["col"] + high
            for key in ("fg", "bg"):
                assert len(run[key]) == 7 and run[key][0] == "#", f"row {r}: {key} {run[key]}"
        assert least <= grid["cols"], f"row {r}: runs past the last column"
        # --text trims every trailing space; --json keeps those that show.
        assert "".join(run["text"] for run in runs).rstrip(" ") == row, f"row {r}: text differs from --text"


def main():
    failed = False
    for json_path in sorted(pathlib.Path(sys.argv[1]).glob("*.json")):
        try:
            check(json_path)
        except (AssertionError, KeyError, ValueError) as error:
            print(f"FAIL {json_path}: {error}")
            failed = True
    sys.exit(1 if failed else 0)


main()
