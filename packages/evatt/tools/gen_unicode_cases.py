"""Freeze multi-character Unicode 16 normalisation and casing regressions.

python3.14 tools/gen_unicode_cases.py > tests/fixtures/unicode-16.json
These fabricated strings exercise combining order, Hangul composition,
expanding folds and final-sigma context, rather than only individual scalars.
"""

import json
import random
import sys
import unicodedata


def main():
    assert sys.version_info[:2] == (3, 14)
    assert unicodedata.unidata_version == "16.0.0"
    rng = random.Random(0xE0A77)
    marks = [chr(cp) for cp in range(0x110000) if unicodedata.category(chr(cp)).startswith("M")]
    starters = "AaÉéİıΣσςKſßΐẞÅ\u1100\u1161\u11a8\U0001d15e"
    texts = ["AΣ", "AΣ'A", "AΣ'", "Σ\u0301", "A\u0301Σ\u0301", "\u1100\u1161\u11a8"]
    # A fixed seed makes reproducible fabricated test text; nothing here is secret.
    texts += [rng.choice(starters) + mark + rng.choice(marks) for mark in marks]  # NOSONAR
    for _ in range(2048):
        text = "".join(
            # The same seeded, non-security generator as above.
            rng.choice(starters + "' .\u200d") + rng.choice(marks) for _ in range(rng.randint(1, 8))  # NOSONAR
        )
        texts.append(unicodedata.normalize("NFD", text))
    document = {
        "interpreter": sys.version.split()[0],
        "unidata_version": unicodedata.unidata_version,
        "seed": 0xE0A77,
        "cases": [
            [s, unicodedata.normalize("NFC", s), s.lower(), s.upper(), s.casefold()] for s in texts
        ],
    }
    print(json.dumps(document, ensure_ascii=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
