import csv
import io
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from .errors import SchemaError


@contextmanager
def _csv_source(text: str, path: Path, label: str) -> Iterator[io.StringIO]:
    with io.StringIO(text, newline="") as source:
        try:
            yield source
        except csv.Error as exc:
            raise SchemaError(f"{path}: {label} is not valid CSV: {exc}.") from exc
