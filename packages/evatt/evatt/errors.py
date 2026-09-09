from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .redact import Unknown


class EvattError(ValueError):
    """Input was outside what the pseudonymisation boundary accepts."""


class Halt(EvattError):
    """Redaction stopped because the residual sweep found unclassified candidates.

    Carrying the unknowns on the exception is what lets the CLI write a triage
    file without the redaction pass knowing anything about the filesystem.

    Halt subclasses EvattError, so the order of the except blocks in any caller
    is load-bearing. Catch Halt first and keep it in its own block. A broad
    ``except EvattError`` placed ahead of it swallows the halt and reports it as
    an ordinary error with no triage file written, which is the one failure this
    class exists to prevent.
    """

    def __init__(self, unknowns: tuple[Unknown, ...]) -> None:
        super().__init__(f"{len(unknowns)} unclassified candidate(s); nothing was written")
        self.unknowns = unknowns
