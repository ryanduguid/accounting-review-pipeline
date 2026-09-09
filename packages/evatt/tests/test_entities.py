import json

import pytest

from evatt import entities
from evatt.errors import EvattError

SAMPLE = {
    "schema_version": 1,
    "entries": [
        {"value": "Sample Holdings Pty Ltd", "placeholder": "CLIENT_01",
         "kind": "client", "added": "2026-09-09"},
        {"value": "Jane Roe", "placeholder": "PERSON_01",
         "kind": "person", "added": "2026-09-09"},
    ],
}


def write_map(tmp_path, document):
    path = tmp_path / "entities.json"
    path.write_text(json.dumps(document), encoding="utf-8")
    return path


def test_load_returns_entities_in_file_order(tmp_path) -> None:
    loaded = entities.load(write_map(tmp_path, SAMPLE))
    assert [e.placeholder for e in loaded] == ["CLIENT_01", "PERSON_01"]
    assert loaded[0].value == "Sample Holdings Pty Ltd"
    assert loaded[0].kind == "client"


def test_load_rejects_an_unknown_schema_version(tmp_path) -> None:
    document = dict(SAMPLE, schema_version=2)
    with pytest.raises(EvattError):
        entities.load(write_map(tmp_path, document))


def test_load_rejects_an_unknown_kind(tmp_path) -> None:
    document = {"schema_version": 1, "entries": [
        {"value": "X", "placeholder": "CLIENT_01", "kind": "supplier", "added": "2026-09-09"}]}
    with pytest.raises(EvattError):
        entities.load(write_map(tmp_path, document))


def test_load_rejects_a_placeholder_that_does_not_match_its_kind(tmp_path) -> None:
    document = {"schema_version": 1, "entries": [
        {"value": "X", "placeholder": "PERSON_01", "kind": "client", "added": "2026-09-09"}]}
    with pytest.raises(EvattError):
        entities.load(write_map(tmp_path, document))


def test_load_rejects_duplicate_placeholders(tmp_path) -> None:
    document = {"schema_version": 1, "entries": [
        {"value": "A", "placeholder": "CLIENT_01", "kind": "client", "added": "2026-09-09"},
        {"value": "B", "placeholder": "CLIENT_01", "kind": "client", "added": "2026-09-09"}]}
    with pytest.raises(EvattError):
        entities.load(write_map(tmp_path, document))


def test_load_rejects_duplicate_values(tmp_path) -> None:
    document = {"schema_version": 1, "entries": [
        {"value": "A", "placeholder": "CLIENT_01", "kind": "client", "added": "2026-09-09"},
        {"value": "A", "placeholder": "CLIENT_02", "kind": "client", "added": "2026-09-09"}]}
    with pytest.raises(EvattError):
        entities.load(write_map(tmp_path, document))


def test_assign_numbers_within_a_kind(tmp_path) -> None:
    """Ordinals run per kind, so a new client does not consume a person number."""
    loaded = entities.load(write_map(tmp_path, SAMPLE))
    assert entities.assign(loaded, "Second Client", "client", "2026-09-10").placeholder == "CLIENT_02"
    assert entities.assign(loaded, "Someone Else", "person", "2026-09-10").placeholder == "PERSON_02"


def test_assign_is_stable_for_the_same_input(tmp_path) -> None:
    loaded = entities.load(write_map(tmp_path, SAMPLE))
    first = entities.assign(loaded, "Second Client", "client", "2026-09-10")
    second = entities.assign(loaded, "Second Client", "client", "2026-09-10")
    assert first == second


def test_save_then_load_round_trips(tmp_path) -> None:
    path = tmp_path / "entities.json"
    original = entities.load(write_map(tmp_path, SAMPLE))
    entities.save(path, original)
    assert entities.load(path) == original


def bound_the_walk(monkeypatch, root):
    """Stop the .gitignore walk at ``root``.

    The real walk climbs to the filesystem root, so an unrelated .gitignore
    anywhere above the pytest temporary directory would decide the outcome.
    Bounding it keeps these tests about the tree the test itself built.
    """
    def walk(map_path):
        for directory in [map_path.parent, *map_path.parent.parents]:
            candidate = directory / ".gitignore"
            if candidate.exists():
                yield candidate
            if directory == root:
                return

    monkeypatch.setattr(entities, "_ancestor_gitignores", walk)


def test_require_gitignored_rejects_an_untracked_directory(tmp_path, monkeypatch) -> None:
    bound_the_walk(monkeypatch, tmp_path)
    target = tmp_path / "entities.json"
    target.write_text("{}", encoding="utf-8")
    with pytest.raises(EvattError):
        entities.require_gitignored(target)


def test_require_gitignored_accepts_a_covered_directory(tmp_path, monkeypatch) -> None:
    bound_the_walk(monkeypatch, tmp_path)
    (tmp_path / ".gitignore").write_text("entities.json\n", encoding="utf-8")
    target = tmp_path / "entities.json"
    target.write_text("{}", encoding="utf-8")
    entities.require_gitignored(target)


def test_require_gitignored_rejects_a_commented_out_rule(tmp_path, monkeypatch) -> None:
    bound_the_walk(monkeypatch, tmp_path)
    (tmp_path / ".gitignore").write_text("# entities.json\n", encoding="utf-8")
    target = tmp_path / "entities.json"
    target.write_text("{}", encoding="utf-8")
    with pytest.raises(EvattError):
        entities.require_gitignored(target)


def test_require_gitignored_rejects_a_commented_out_wildcard(tmp_path, monkeypatch) -> None:
    bound_the_walk(monkeypatch, tmp_path)
    (tmp_path / ".gitignore").write_text("#*.entities.json\n", encoding="utf-8")
    target = tmp_path / "acme.entities.json"
    target.write_text("{}", encoding="utf-8")
    with pytest.raises(EvattError):
        entities.require_gitignored(target)


def test_require_gitignored_accepts_a_rule_among_comments_and_blanks(
    tmp_path, monkeypatch
) -> None:
    bound_the_walk(monkeypatch, tmp_path)
    (tmp_path / ".gitignore").write_text(
        "# local secrets\n\n  # entities.json was here\n\n  entities.json  \n\n# end\n",
        encoding="utf-8",
    )
    target = tmp_path / "entities.json"
    target.write_text("{}", encoding="utf-8")
    entities.require_gitignored(target)


def test_require_gitignored_accepts_the_wildcard_for_a_suffixed_map(tmp_path, monkeypatch) -> None:
    bound_the_walk(monkeypatch, tmp_path)
    (tmp_path / ".gitignore").write_text("# maps\n*.entities.json\n", encoding="utf-8")
    target = tmp_path / "acme.entities.json"
    target.write_text("{}", encoding="utf-8")
    entities.require_gitignored(target)
