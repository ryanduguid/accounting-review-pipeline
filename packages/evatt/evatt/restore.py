"""Reverses the entity map over a model's answer, locally.

Named entities only. Structured identifiers were replaced one-way and no
record of their values exists, so a TFN, ABN, ACN, BSB or Medicare number
cannot come back through here however the answer is worded.
"""
from __future__ import annotations

import re
from typing import Callable, Sequence

from .entities import Entity


def _literally(value: str) -> Callable[[re.Match[str]], str]:
    """A replacement function that writes *value* exactly as it stands.

    ``re.sub`` reads backslashes and ``\\g`` in a string replacement as
    references. A real value is arbitrary text off the map, so "Smith \\ Co"
    would raise, and a value holding "\\1" would expand to a group that is not
    there. A function replacement is never scanned for escapes.
    """
    return lambda _match: value


def restore(text: str, entities: Sequence[Entity]) -> str:
    """Substitute each placeholder back to its real value.

    The trailing ``(?!\\d)`` is load-bearing. Without it, replacing CLIENT_10
    also rewrites the first nine characters of CLIENT_100, and the map holds
    both the moment a client list passes ninety-nine entries.
    """
    for entity in entities:
        pattern = re.compile(re.escape(entity.placeholder) + r"(?!\d)")
        text = pattern.sub(_literally(entity.value), text)
    return text
