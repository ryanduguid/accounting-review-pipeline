"""Document intake and bundle refusals the main document tests do not reach."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from reviewready.documents import (
    INTAKE_NAME,
    REVIEW_NAME,
    create_intake,
    load_document,
    render_document,
)
from reviewready.errors import GateInputError
from tests.test_documents import ENTITY, TEXT, bundle_at, reviewed, save_review


def _intake(tmp_path: Path, source: Path, **overrides) -> Path:
    arguments = dict(source_path=source, text_path=source, output_dir=tmp_path / "bundle",
                     entity=ENTITY, period_end="2026-03-31", text_origin="manual")
    arguments.update(overrides)
    return create_intake(**arguments)


def test_unreadable_source_is_named(tmp_path: Path) -> None:
    folder = tmp_path / "folder.txt"
    folder.mkdir()
    with pytest.raises(GateInputError, match="Cannot read document input"):
        _intake(tmp_path, folder)
    assert not (tmp_path / "bundle").exists()  # nosec B101: pytest outcome assertion


@pytest.mark.parametrize(
    "suffix,origin,match",
    [
        (".docx", "manual", "Original documents must be .txt or .pdf files"),
        (".txt", "ocr", "text_origin must be manual or pdf-inspector"),
    ],
)
def test_unsupported_source_type_or_text_origin_is_refused(
    tmp_path: Path, suffix: str, origin: str, match: str
) -> None:
    source = tmp_path / f"invoice{suffix}"
    source.write_text(TEXT, encoding="utf-8")
    with pytest.raises(GateInputError, match=match):
        _intake(tmp_path, source, text_origin=origin)
    assert not (tmp_path / "bundle").exists()  # nosec B101: pytest outcome assertion


def test_observations_over_the_intake_limit_are_refused(tmp_path: Path) -> None:
    # Each short labelled line becomes a full observation record, so text well
    # under its own 1 MiB limit can still exceed the manifest limit.
    source = tmp_path / "source.txt"
    source.write_text("Total: 1\n" * 40000, encoding="utf-8")
    with pytest.raises(GateInputError, match="exceed the 1 MiB intake limit"):
        _intake(tmp_path, source)
    assert not (tmp_path / "bundle").exists()  # nosec B101: pytest outcome assertion


def test_bundle_path_that_cannot_be_resolved_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bundle = bundle_at(tmp_path)

    def unresolvable(self: Path, strict: bool = False) -> Path:
        raise OSError("resolution failed")

    with monkeypatch.context() as patch:
        patch.setattr(type(bundle), "resolve", unresolvable)
        with pytest.raises(GateInputError, match=f"Cannot resolve document input {INTAKE_NAME}"):
            load_document(bundle)


@pytest.mark.parametrize(
    "content,match",
    [("{", "is not valid document JSON"), ("[]", "must hold a JSON object")],
)
def test_intake_that_is_not_a_json_object_is_refused(
    tmp_path: Path, content: str, match: str
) -> None:
    bundle = bundle_at(tmp_path)
    (bundle / INTAKE_NAME).write_text(content, encoding="utf-8")
    with pytest.raises(GateInputError, match=f"{INTAKE_NAME} {match}"):
        load_document(bundle)


def test_unsigned_review_template_still_needs_a_reviewer(tmp_path: Path) -> None:
    bundle = bundle_at(tmp_path)
    template = json.loads((bundle / "document-review.example.json").read_text(encoding="utf-8"))
    save_review(bundle, template)
    issues = load_document(bundle).issues
    assert issues[0] == (  # nosec B101: pytest outcome assertion
        "Record the reviewer, date and confirmation against the original document."
    )


def test_review_value_without_a_page_is_refused(tmp_path: Path) -> None:
    bundle = bundle_at(tmp_path)
    review = reviewed(bundle)
    review["fields"]["total"]["page"] = None
    save_review(bundle, review)
    with pytest.raises(GateInputError, match="total.page must identify a page"):
        load_document(bundle)


def test_field_with_no_observation_is_shown_as_such(tmp_path: Path) -> None:
    bundle = bundle_at(tmp_path, text=TEXT.replace("GST: 10.00", "No tax details"))
    rendered = render_document(load_document(bundle))
    gst = rendered.split("\nGST:\n", 1)[1]
    assert gst.startswith("  no observation\n")  # nosec B101: pytest outcome assertion
    assert not (bundle / REVIEW_NAME).exists()  # nosec B101: pytest outcome assertion
