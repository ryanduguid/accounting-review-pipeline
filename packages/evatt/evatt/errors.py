from __future__ import annotations


class EvattError(ValueError):
    """Input was outside what the pseudonymisation boundary accepts."""


class Halt(EvattError):
    """Redaction stopped because the residual sweep found unclassified candidates.

    Carrying the unknowns on the exception is what lets the CLI write a triage
    file without the redaction pass knowing anything about the filesystem.
    """

    def __init__(self, unknowns: tuple[object, ...]) -> None:
        super().__init__(f"{len(unknowns)} unclassified candidate(s); nothing was written")
        self.unknowns = unknowns
