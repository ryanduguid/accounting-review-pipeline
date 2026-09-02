class GateInputError(ValueError):
    """Raised when an input cannot safely support a readiness decision."""


class SchemaError(GateInputError):
    """An input file breaks its structural contract: headers, row shape, field content."""


class DuplicateKeyError(GateInputError):
    """An input file repeats a key that the gate requires to be unique."""


class DateMismatchError(GateInputError):
    """Dates inside or across inputs disagree with the period the run claims to gate."""


class NumericGateError(GateInputError):
    """A monetary value cannot be read as an exact, finite decimal."""
