class ControlInputError(ValueError):
    """Raised when an input cannot safely support a close-control decision."""


class SchemaError(ControlInputError):
    """An input file breaks its structural contract: headers, row shape, field content."""


class DuplicateKeyError(ControlInputError):
    """An input file repeats a key that the control model requires to be unique."""


class DateMismatchError(ControlInputError):
    """Dates inside or across inputs disagree with the period the run claims to review."""


class NumericGateError(ControlInputError):
    """A monetary value cannot be read as an exact, finite decimal."""
