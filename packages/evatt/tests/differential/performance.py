"""Measure the original timing families at 1, 2, 4, 8 and 16 times size.

python3.14 tests/differential/performance.py --probe target/release/evatt-probe
No global linearity claim: map size, arbitration and sorting are outside the
matcher bounds. Name counters count real candidate starts and retries; number
counters count actual cursor inspections, excluding regex token classification.
"""

import argparse
import json
import subprocess
from pathlib import Path


def cases():
    result = []
    for label, pattern in [
        ("TFN", "TFN_LABELLED"),
        ("ABN", "ABN_LABELLED"),
        ("ACN", "ACN_LABELLED"),
        ("Medicare", "MEDICARE_LABELLED"),
    ]:
        result.append(
            {
                "family": "label",
                "pattern": pattern,
                "prefix": label,
                "unit": " ",
                "suffix": "",
                "base": 20000,
            }
        )
        for padding in ["", " \t"]:
            # Keep the original two long markup runs and intervening spaces.
            result.append(
                {
                    "family": "label",
                    "pattern": pattern,
                    "prefix": label,
                    "unit": "*_" + padding + "`",
                    "suffix": ":x",
                    "base": 5000,
                    "markup": True,
                    "padding": padding,
                }
            )
    for unit in ["Tax Court ", "Tax Zzz ", "Income Tax\nJohn Smith\n"]:
        result.append({"family": "name", "prefix": "", "unit": unit, "suffix": "", "base": 2000})
    for value, prefix, unit in [
        ("A * B", "A", "*"),
        ("A B", "A ", "<br\n>"),
        ("A <br B", "A ", "<br "),
        ("A B", "A", "\n> "),
        ("A B", "A", "\u200b_*"),
    ]:
        result.append(
            {
                "family": "value",
                "value": value,
                "prefix": prefix,
                "unit": unit,
                "suffix": "X",
                "base": 5000,
            }
        )
    for unit in ["9", "9-", "9.,", "9\t ", "-", " "]:
        result.append({"family": "number", "prefix": "", "unit": unit, "suffix": "", "base": 5000})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", required=True)
    parser.add_argument("--output")
    args = parser.parse_args()
    batch = [
        {**case, "mode": "benchmark", "size": case["base"] * factor, "factor": factor}
        for case in cases()
        for factor in [1, 2, 4, 8, 16]
    ]
    completed = subprocess.run(
        [args.probe],
        input="".join(json.dumps(r) + "\n" for r in batch),
        capture_output=True,
        text=True,
        check=True,
    )
    answers = [json.loads(line) for line in completed.stdout.splitlines()]
    assert len(answers) == len(batch)
    assert all("error" not in answer for answer in answers), answers
    rows = [{**request, **answer} for request, answer in zip(batch, answers)]
    for group in [rows[i : i + 5] for i in range(0, len(rows), 5)]:
        growth = group[-1]["seconds"] / max(group[0]["seconds"], 1e-9)
        print(
            group[0]["family"],
            repr(group[0]["unit"]),
            [round(r["seconds"], 6) for r in group],
            "16x growth",
            round(growth, 2),
        )
        assert group[-1]["seconds"] < group[0]["seconds"] * 32 + 0.01
        if group[0]["family"] == "number":
            assert group[-1]["characters_examined"] <= group[0]["characters_examined"] * 16 + 32
        if group[0]["family"] == "name":
            assert group[-1]["candidate_starts"] <= group[0]["candidate_starts"] * 16 + 16
            assert group[-1]["retries"] <= group[0]["retries"] * 16 + 16
    if args.output:
        Path(args.output).write_text(json.dumps(rows, indent=2) + "\n")
    print(f"{len(rows)} release measurements; bounded growth checks passed")


if __name__ == "__main__":
    main()
