"""Check that each <log>.json in a directory parses, has the shape --json
promises, and spells the rows of <log>.txt beside it. Run by test.sh.

    python3 tests/grids/check.py tests/grids
"""
import json
import pathlib
import sys
import unicodedata


def width(text):
    return sum(2 if unicodedata.east_asian_width(ch) in "WF" else 1 for ch in text)


def check(json_path):
    grid = json.loads(json_path.read_text(encoding="utf-8"))
    rows = json_path.with_suffix(".txt").read_text(encoding="utf-8").split("\n")[:-1]
    assert len(grid["lines"]) == grid["rows"] == len(rows), "row count"
    cursor = grid["cursor"]
    if cursor is not None:
        assert 0 <= cursor["col"] < grid["cols"] and 0 <= cursor["row"] < grid["rows"], "cursor off the grid"
    for r, (runs, row) in enumerate(zip(grid["lines"], rows)):
        col = 0
        for run in runs:
            assert run["col"] >= col, f"row {r}: runs overlap"
            col = run["col"] + width(run["text"])
            for key in ("fg", "bg"):
                assert len(run[key]) == 7 and run[key][0] == "#", f"row {r}: {key} {run[key]}"
        assert col <= grid["cols"], f"row {r}: runs past the last column"
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
