import json
import os
import re
import shutil
import subprocess
from pathlib import Path

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

# 2026-09-09 written in Arabic-Indic digits, which the old \d date regex accepted.
ARABIC_INDIC_DATE = "2026-09-09".translate({ord(str(n)): chr(0x0660 + n) for n in range(10)})


def write_map(tmp_path, document):
    path = tmp_path / "entities.json"
    path.write_text(json.dumps(document), encoding="utf-8")
    return path


git_required = pytest.mark.skipif(shutil.which("git") is None, reason="git is not installed")


@pytest.fixture(autouse=True)
def isolated_git(tmp_path, monkeypatch):
    """Keep the developer's own git configuration out of the guard tests.

    A global core.excludesFile could otherwise decide whether a map counts as
    ignored, and an ancestor of the pytest directory could pose as a repository.
    """
    monkeypatch.setenv("GIT_CONFIG_GLOBAL", str(tmp_path / "absent-gitconfig"))
    monkeypatch.setenv("GIT_CONFIG_SYSTEM", str(tmp_path / "absent-gitconfig"))
    monkeypatch.setenv("GIT_CEILING_DIRECTORIES", str(tmp_path.parent))


def git(cwd, *args):
    subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True)


def new_repo(tmp_path, gitignore=None):
    """Build a throwaway repository holding an entity map, and return its path."""
    repo = tmp_path / "repo"
    repo.mkdir()
    git(repo, "init", "-q")
    if gitignore is not None:
        (repo / ".gitignore").write_text(gitignore, encoding="utf-8")
    target = repo / "entities.json"
    target.write_text("{}", encoding="utf-8")
    return target


def repo_map(tmp_path, document, gitignore="entities.json\n*.tmp\n"):
    """A real map inside a repository. save guards the .tmp, so it needs one."""
    target = new_repo(tmp_path, gitignore)
    target.write_text(json.dumps(document), encoding="utf-8")
    return target


def test_load_returns_entities_in_file_order(tmp_path) -> None:
    loaded = entities.load(write_map(tmp_path, SAMPLE))
    assert [e.placeholder for e in loaded] == ["CLIENT_01", "PERSON_01"]
    assert loaded[0].value == "Sample Holdings Pty Ltd"
    assert loaded[0].kind == "client"


def test_load_rejects_an_unknown_schema_version(tmp_path) -> None:
    document = dict(SAMPLE, schema_version=2)
    with pytest.raises(EvattError):
        entities.load(write_map(tmp_path, document))


@pytest.mark.parametrize("version", [True, 1.0])
def test_load_rejects_a_schema_version_that_merely_equals_one(tmp_path, version) -> None:
    """True == 1 and 1.0 == 1, so equality alone lets a wrong type through."""
    with pytest.raises(EvattError):
        entities.load(write_map(tmp_path, dict(SAMPLE, schema_version=version)))


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


def test_load_rejects_a_single_digit_ordinal(tmp_path) -> None:
    """CLIENT_1 beside CLIENT_10 would let naive replacement corrupt the longer one."""
    document = {"schema_version": 1, "entries": [
        {"value": "A", "placeholder": "CLIENT_1", "kind": "client", "added": "2026-09-09"},
        {"value": "B", "placeholder": "CLIENT_10", "kind": "client", "added": "2026-09-09"}]}
    with pytest.raises(EvattError):
        entities.load(write_map(tmp_path, document))


@pytest.mark.parametrize("added", ["2026-13-45", ARABIC_INDIC_DATE, "20260909", "2026-W01-1"])
def test_load_rejects_a_date_that_is_not_a_real_calendar_day(tmp_path, added) -> None:
    document = {"schema_version": 1, "entries": [
        {"value": "A", "placeholder": "CLIENT_01", "kind": "client", "added": added}]}
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


def test_assign_never_reuses_an_ordinal_freed_by_a_deletion(tmp_path) -> None:
    """CLIENT_02 may already stand for a deleted client in a redacted document."""
    document = {"schema_version": 1, "entries": [
        {"value": "A", "placeholder": "CLIENT_01", "kind": "client", "added": "2026-09-09"},
        {"value": "C", "placeholder": "CLIENT_03", "kind": "client", "added": "2026-09-09"}]}
    loaded = entities.load(write_map(tmp_path, document))
    assert entities.assign(loaded, "D", "client", "2026-09-10").placeholder == "CLIENT_04"


def test_assign_returns_the_existing_entity_for_a_known_value(tmp_path) -> None:
    """A second placeholder for one real value would fail load's duplicate check."""
    loaded = entities.load(write_map(tmp_path, SAMPLE))
    again = entities.assign(loaded, "Sample Holdings Pty Ltd", "client", "2026-09-10")
    assert again is loaded[0]


@pytest.mark.parametrize(
    ("value", "kind", "added"),
    [("", "client", "2026-09-10"), ("   ", "client", "2026-09-10"),
     ("A", "client", "not-a-date"), ("A", "client", "2026-13-45"), ("A", "supplier", "2026-09-10")],
)
def test_assign_refuses_what_load_would_reject(value, kind, added) -> None:
    with pytest.raises(EvattError):
        entities.assign([], value, kind, added)


def test_assign_is_stable_for_the_same_input(tmp_path) -> None:
    loaded = entities.load(write_map(tmp_path, SAMPLE))
    first = entities.assign(loaded, "Second Client", "client", "2026-09-10")
    second = entities.assign(loaded, "Second Client", "client", "2026-09-10")
    assert first == second


@git_required
def test_save_then_load_round_trips(tmp_path) -> None:
    path = repo_map(tmp_path, SAMPLE)
    original = entities.load(path)
    entities.save(path, original)
    assert entities.load(path) == original


@git_required
def test_save_writes_the_entries_it_validated(tmp_path) -> None:
    path = repo_map(tmp_path, SAMPLE)
    original = entities.load(path)

    class ChangingSequence(list):
        def __iter__(self):
            snapshot = tuple(super().__iter__())
            self[:] = [entities.Entity("Jane Roe", "TFN_01", "person", "2026-09-09")]
            return iter(snapshot)

    entities.save(path, ChangingSequence(original))
    assert entities.load(path) == original


@git_required
@pytest.mark.parametrize("tracked", [False, True])
def test_save_refuses_a_committable_destination(tmp_path, tracked) -> None:
    path = repo_map(tmp_path, SAMPLE, gitignore="*.tmp\n")
    if tracked:
        git(path.parent, "add", "entities.json")
    before = path.read_bytes()
    with pytest.raises(EvattError, match="must never be committed"):
        entities.save(path, entities.load(path)[:1])
    assert path.read_bytes() == before
    assert not path.with_name(path.name + ".tmp").exists()


@git_required
def test_save_through_a_symlink_updates_the_ignored_target(tmp_path) -> None:
    path = repo_map(tmp_path, SAMPLE)
    alias = path.with_name("map-link.json")
    try:
        alias.symlink_to(path)
    except OSError:
        pytest.skip("creating symbolic links is unavailable")
    updated = entities.load(path)[:1]
    entities.save(alias, updated)
    assert alias.is_symlink()
    assert entities.load(path) == updated


@git_required
def test_save_leaves_the_previous_map_intact_when_the_write_fails(tmp_path, monkeypatch) -> None:
    """An interrupted save must not truncate the only copy of the key."""
    path = repo_map(tmp_path, SAMPLE)
    before = path.read_text(encoding="utf-8")

    def fail(source, destination):
        raise OSError("interrupted")

    monkeypatch.setattr(entities.os, "replace", fail)
    with pytest.raises(OSError):
        entities.save(path, entities.load(path)[:1])
    assert path.read_text(encoding="utf-8") == before
    # The temporary holds the same real values, so it must not survive the failure.
    assert list(path.parent.glob("*.tmp")) == []


@git_required
def test_save_refuses_a_temporary_git_would_let_you_commit(tmp_path) -> None:
    """A rule covering the map does not cover its .tmp, which holds the same values.

    The refusal has to name the temporary, not the map: the map here is ignored.
    """
    path = repo_map(tmp_path, SAMPLE, gitignore="entities.json\n")
    before = path.read_text(encoding="utf-8")
    with pytest.raises(EvattError, match=re.escape("entities.json.tmp")):
        entities.save(path, entities.load(path)[:1])
    assert path.read_text(encoding="utf-8") == before
    assert list(path.parent.glob("*.tmp")) == []


def fixed_tokens(monkeypatch, *tokens):
    """Make the temporary's name predictable, so a test can lie in wait at it."""
    minted = iter(tokens)
    monkeypatch.setattr(entities.secrets, "token_hex", lambda _bytes: next(minted))


def temporary_named(path, token):
    return path.with_name(f"{path.name}.tmp.{token}.tmp")


@git_required
@pytest.mark.parametrize("kind", ["symbolic", "hard"])
def test_a_link_at_the_temporary_path_is_never_written_through(
    tmp_path, monkeypatch, kind
) -> None:
    """The temporary is created exclusively, so a name that already exists is not opened.

    ``save`` used to open the fixed name ``entities.json.tmp`` in write mode.
    A symbolic or hard link sitting there, at a name in a directory the
    operator's own tools write to, was followed and its target truncated, and
    the map was then moved on top of the overwritten file. Both files are
    ignored by construction, which is why nobody would see it: the destroyed
    one never appears in a diff.

    The victim here is another ignored file, which is the case in the report.
    """
    path = repo_map(tmp_path, SAMPLE)
    victim = path.with_name("other-ignored.tmp")
    victim.write_text("not the map\n", encoding="utf-8")
    ambush = temporary_named(path, "aaaaaaaa")
    try:
        if kind == "symbolic":
            ambush.symlink_to(victim)
        else:
            os.link(victim, ambush)
    except OSError:
        pytest.skip(f"creating a {kind} link is unavailable")
    fixed_tokens(monkeypatch, "aaaaaaaa", "bbbbbbbb")

    updated = entities.load(path)[:1]
    entities.save(path, updated)

    assert victim.read_text(encoding="utf-8") == "not the map\n"
    assert entities.load(path) == updated
    # The ambush is left exactly as it was found: not written through, not
    # removed, and not mistaken for this run's own temporary.
    assert ambush.exists()
    assert sorted(p.name for p in path.parent.glob("*.tmp")) == [ambush.name, victim.name]


@git_required
def test_save_stops_when_the_temporary_name_cannot_be_created(tmp_path, monkeypatch) -> None:
    """Every attempt blocked is a refusal, never a write through whatever is there."""
    path = repo_map(tmp_path, SAMPLE)
    before = path.read_bytes()
    occupied = temporary_named(path, "aaaaaaaa")
    occupied.write_text("somebody else's file\n", encoding="utf-8")
    fixed_tokens(monkeypatch, *["aaaaaaaa"] * entities._TEMPORARY_ATTEMPTS)

    with pytest.raises(EvattError, match="attempts"):
        entities.save(path, entities.load(path)[:1])

    assert path.read_bytes() == before
    assert occupied.read_text(encoding="utf-8") == "somebody else's file\n"


@git_required
def test_save_refuses_when_a_file_cannot_be_created_exclusively(tmp_path, monkeypatch) -> None:
    """Exclusive creation is the whole protection, so its absence is a refusal.

    No platform CPython supports lacks O_CREAT and O_EXCL. The point of the
    branch is that the day one does, ``save`` stops rather than falling back to
    opening a pathname something else may already own.
    """
    path = repo_map(tmp_path, SAMPLE)
    before = path.read_bytes()
    monkeypatch.setattr(entities, "_EXCLUSIVE_CREATION", False)

    with pytest.raises(EvattError, match="exclusively"):
        entities.save(path, entities.load(path)[:1])

    assert path.read_bytes() == before
    assert list(path.parent.glob("*.tmp")) == []


@git_required
def test_the_temporary_is_removed_whatever_it_was_named(tmp_path, monkeypatch) -> None:
    """A unique name must not mean a leftover copy of the key nobody looks for."""
    path = repo_map(tmp_path, SAMPLE)
    fixed_tokens(monkeypatch, "aaaaaaaa")
    entities.save(path, entities.load(path)[:1])
    assert not temporary_named(path, "aaaaaaaa").exists()
    assert sorted(p.name for p in path.parent.iterdir()) == [".git", ".gitignore", "entities.json"]


@git_required
def test_the_saved_map_is_lf_on_every_platform(tmp_path) -> None:
    """The map is this package's own file, so it holds one encoding of one document."""
    path = repo_map(tmp_path, SAMPLE)
    entities.save(path, entities.load(path))
    assert b"\r\n" not in path.read_bytes()


ENTITY = entities.Entity
REJECTED = [
    # A second spelling of one value, which pass two matches as the first one.
    ((ENTITY("Jane Roe", "PERSON_01", "person", "2026-09-09"),
      ENTITY("JANE  ROE", "PERSON_02", "person", "2026-09-09")), "once case and whitespace"),
    ((ENTITY("Jane Roe", "PERSON_01", "person", "2026-09-09"),
      ENTITY("Jane Roe", "PERSON_02", "person", "2026-09-09")), "duplicate entity value"),
    ((ENTITY("Jane Roe", "PERSON_01", "person", "2026-09-09"),
      ENTITY("John Smith", "PERSON_01", "person", "2026-09-09")), "duplicate placeholder"),
    ((ENTITY("Jane Roe", "CLIENT_01", "person", "2026-09-09"),), "does not match kind"),
    ((ENTITY("Jane Roe", "PERSON_1", "person", "2026-09-09"),), "does not match kind"),
    ((ENTITY("Jane Roe", "", "person", "2026-09-09"),), "does not match kind"),
    ((ENTITY("", "PERSON_01", "person", "2026-09-09"),), "non-empty string"),
    ((ENTITY("TFN_01", "PERSON_01", "person", "2026-09-09"),), "shaped like an assigned"),
    ((ENTITY("Jane Roe", "PERSON_01", "friend", "2026-09-09"),), "unknown entity kind"),
    ((ENTITY("Jane Roe", "PERSON_01", "person", "9 September"),), "must be YYYY-MM-DD"),
]


@git_required
@pytest.mark.parametrize("sequence, message", REJECTED)
def test_save_refuses_a_sequence_load_would_reject(tmp_path, sequence, message) -> None:
    """The map is the only copy of the key, so it is never replaced with an unreadable one.

    ``save`` serialised whatever it was handed. A library caller could
    atomically replace the map with a document every later command fails to
    load, and nothing said so until the next run, by which point the readable
    copy was gone.

    Nothing is created either: the directory holds what it held, so the
    validation runs before a temporary exists rather than after one is cleaned
    up.
    """
    path = repo_map(tmp_path, SAMPLE)
    before = path.read_bytes()
    with pytest.raises(EvattError, match=message):
        entities.save(path, sequence)
    assert path.read_bytes() == before
    assert sorted(p.name for p in path.parent.iterdir()) == [".git", ".gitignore", "entities.json"]


@pytest.mark.parametrize("sequence, message", REJECTED)
def test_load_refuses_what_save_refuses(tmp_path, sequence, message) -> None:
    """The pairing itself, so the two cannot drift into disagreeing about a map."""
    document = {
        "schema_version": 1,
        "entries": [
            {"value": e.value, "placeholder": e.placeholder, "kind": e.kind, "added": e.added}
            for e in sequence
        ],
    }
    with pytest.raises(EvattError, match=message):
        entities.load(write_map(tmp_path, document))


@git_required
def test_require_gitignored_accepts_an_ignored_untracked_map(tmp_path) -> None:
    entities.require_gitignored(new_repo(tmp_path, "entities.json\n"))


@git_required
def test_require_gitignored_accepts_a_wildcard_rule_git_honours(tmp_path) -> None:
    entities.require_gitignored(new_repo(tmp_path, "*.json\n"))


@git_required
def test_require_gitignored_rejects_a_map_no_rule_covers(tmp_path) -> None:
    with pytest.raises(EvattError):
        entities.require_gitignored(new_repo(tmp_path))


@git_required
def test_require_gitignored_rejects_a_tracked_map_listed_in_gitignore(tmp_path) -> None:
    """A .gitignore rule does not ignore an already-tracked file, so the guard must not pass.

    Match the tracked message, not merely EvattError. check-ignore is index-aware
    by default and already exits 1 here, so the ignored branch refuses this file
    on its own and the tracked branch looks redundant. It is one flag from being
    the only thing refusing: check-ignore --no-index exits 0 for a tracked file.
    """
    target = new_repo(tmp_path)
    git(target.parent, "add", "-f", "entities.json")
    (target.parent / ".gitignore").write_text("entities.json\n", encoding="utf-8")
    with pytest.raises(EvattError, match="git already tracks it"):
        entities.require_gitignored(target)


@git_required
def test_require_gitignored_rejects_a_negated_rule(tmp_path) -> None:
    """The last matching rule wins, so the negation leaves the map unignored."""
    with pytest.raises(EvattError):
        entities.require_gitignored(new_repo(tmp_path, "entities.json\n!entities.json\n"))


@git_required
def test_require_gitignored_rejects_a_rule_in_a_gitignore_above_the_repository(tmp_path) -> None:
    """Git never reads a .gitignore outside the work tree, so neither may the guard."""
    (tmp_path / ".gitignore").write_text("entities.json\n", encoding="utf-8")
    with pytest.raises(EvattError):
        entities.require_gitignored(new_repo(tmp_path))


@git_required
def test_require_gitignored_rejects_a_directory_that_is_not_a_repository(tmp_path) -> None:
    target = tmp_path / "entities.json"
    target.write_text("{}", encoding="utf-8")
    with pytest.raises(EvattError):
        entities.require_gitignored(target)


@git_required
def test_require_gitignored_accepts_a_relative_path(tmp_path, monkeypatch) -> None:
    target = new_repo(tmp_path, "entities.json\n")
    monkeypatch.chdir(target.parent)
    entities.require_gitignored(Path("entities.json"))


@git_required
def test_require_gitignored_names_the_resolved_path_when_it_refuses(tmp_path, monkeypatch) -> None:
    """A relative argument must still name the real file, not 'entities.json'."""
    target = new_repo(tmp_path)
    monkeypatch.chdir(target.parent)
    with pytest.raises(EvattError, match=re.escape(str(target.resolve()))):
        entities.require_gitignored(Path("entities.json"))


def test_require_gitignored_fails_closed_when_git_is_missing(tmp_path, monkeypatch) -> None:
    def missing(*args, **kwargs):
        raise FileNotFoundError(2, "No such file or directory", "git")

    monkeypatch.setattr(entities.subprocess, "run", missing)
    with pytest.raises(EvattError):
        entities.require_gitignored(tmp_path / "entities.json")


def test_require_gitignored_fails_closed_when_git_times_out(tmp_path, monkeypatch) -> None:
    def slow(*args, **kwargs):
        raise subprocess.TimeoutExpired(cmd="git", timeout=30)

    monkeypatch.setattr(entities.subprocess, "run", slow)
    with pytest.raises(EvattError):
        entities.require_gitignored(tmp_path / "entities.json")


def test_require_gitignored_gives_every_git_call_a_timeout(tmp_path, monkeypatch) -> None:
    """A hung git would otherwise block the guard, and so the caller, indefinitely."""
    timeouts: list[object] = []

    def record(*args, **kwargs):
        timeouts.append(kwargs.get("timeout"))
        return subprocess.CompletedProcess(args=list(args), returncode=1)

    monkeypatch.setattr(entities.subprocess, "run", record)
    with pytest.raises(EvattError):
        entities.require_gitignored(tmp_path / "entities.json")
    assert len(timeouts) == 2
    assert all(isinstance(t, (int, float)) and t > 0 for t in timeouts)


def test_require_gitignored_fails_closed_on_an_unexpected_exit_code(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(entities, "_git", lambda subcommand, target: 3)
    with pytest.raises(EvattError):
        entities.require_gitignored(tmp_path / "entities.json")


@pytest.mark.parametrize(
    "value",
    ["TFN_01", "CLIENT_07", "Acme ENTITY_12 Pty Ltd",
     "tfn_01", "Tfn_01", "client_01", "medicare_01", "Acme entity_12 Pty Ltd"],
)
def test_a_placeholder_shaped_value_never_enters_the_map(value, tmp_path) -> None:
    """Pass two compiles the raw value into a pattern, so such a value destroys a real one.

    A map holding "TFN_01" rewrites the placeholder pass one has just written
    over a real tax file number, and leaves a manifest still counting the tfn.

    The lower-case and mixed-case spellings are here because ``value_pattern``
    is ``re.IGNORECASE``: "tfn_01" compiles into a pattern that matches TFN_01
    just as surely as "TFN_01" does. While this guard read the case-sensitive
    PLACEHOLDER it accepted every one of them, and ``load`` accepted them too,
    so the damage was reachable from a map file on disk and not only from a
    hand-built Sequence.
    """
    with pytest.raises(EvattError):
        entities.assign([], value, "client", "2026-09-10")
    document = {"schema_version": 1, "entries": [
        {"value": value, "placeholder": "CLIENT_01", "kind": "client", "added": "2026-09-09"}]}
    with pytest.raises(EvattError):
        entities.load(write_map(tmp_path, document))


def test_a_value_that_merely_looks_placeholder_ish_is_still_accepted() -> None:
    """The guard must not cost a real client whose name happens to end in digits."""
    assert entities.assign([], "Precinct 88", "client", "2026-09-10").value == "Precinct 88"
    assert entities.assign([], "MY_CLIENT_01", "client", "2026-09-10").value == "MY_CLIENT_01"


def test_load_rejects_two_values_that_fold_onto_one_another(tmp_path) -> None:
    """Two spellings of one name cannot hold two placeholders.

    Pass two matches a value case-insensitively and across a whitespace run, so
    "Jane Roe" and "jane  roe" are one pattern. A map holding both would have
    one of them take every occurrence while the other sat there looking
    assigned and never appearing, which is how a restore puts a wrong name back.
    """
    for second in ("jane roe", "JANE ROE", "Jane  Roe", "Jane\nRoe", " Jane Roe "):
        document = {
            "schema_version": 1,
            "entries": [
                dict(SAMPLE["entries"][1]),
                {"value": second, "placeholder": "PERSON_02",
                 "kind": "person", "added": "2026-09-09"},
            ],
        }
        with pytest.raises(EvattError, match="once case and whitespace are folded"):
            entities.load(write_map(tmp_path, document))


def test_load_still_calls_an_exact_duplicate_a_duplicate(tmp_path) -> None:
    """The fold check subsumes the exact one, so it must keep the exact wording."""
    entry = dict(SAMPLE["entries"][1])
    document = {
        "schema_version": 1,
        "entries": [entry, {**entry, "placeholder": "PERSON_02"}],
    }
    with pytest.raises(EvattError, match="duplicate entity value"):
        entities.load(write_map(tmp_path, document))


def test_assign_returns_the_existing_entity_for_a_folded_spelling() -> None:
    """An operator types what the document showed them, which may be shouted or wrapped.

    Minting a second placeholder there would put two on one person, and the
    second would never appear in a document because a single pattern already
    matched every occurrence.
    """
    existing = entities.Entity("Jane Roe", "PERSON_01", "person", "2026-09-09")
    for typed in ("Jane Roe", "jane roe", "JANE ROE", "Jane  Roe", "Jane\nRoe"):
        assert entities.assign((existing,), typed, "person", "2026-09-10") is existing


def test_assign_still_mints_a_new_placeholder_for_a_different_person() -> None:
    existing = entities.Entity("Jane Roe", "PERSON_01", "person", "2026-09-09")
    minted = entities.assign((existing,), "Jane Rowe", "person", "2026-09-10")
    assert minted.placeholder == "PERSON_02"


def test_the_shipped_sample_map_holds_no_two_values_that_fold_together() -> None:
    """The map this package ships has to satisfy the rule load now enforces."""
    sample = Path(entities.__file__).resolve().parent / "samples" / "entities.sample.json"
    loaded = entities.load(sample)
    folds = [entities._fold(entity.value) for entity in loaded]
    assert len(set(folds)) == len(folds)
