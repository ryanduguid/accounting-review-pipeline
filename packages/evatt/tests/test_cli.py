import json
import shutil
import subprocess
from pathlib import Path

import pytest

from evatt import cli
from evatt import entities as entities_module
from evatt.cli import main

SAMPLES = Path(entities_module.__file__).resolve().parent / "samples"

# Every command calls require_gitignored before it reads the map, so the whole
# file needs a real git.
pytestmark = pytest.mark.skipif(shutil.which("git") is None, reason="git is not installed")


@pytest.fixture(autouse=True)
def isolated_git(tmp_path, monkeypatch):
    """Keep the developer's own git configuration out of the guard tests.

    A global core.excludesFile could otherwise decide whether a map counts as
    ignored, and an ancestor of the pytest directory could pose as a repository.
    """
    monkeypatch.setenv("GIT_CONFIG_GLOBAL", str(tmp_path / "absent-gitconfig"))
    monkeypatch.setenv("GIT_CONFIG_SYSTEM", str(tmp_path / "absent-gitconfig"))
    monkeypatch.setenv("GIT_CEILING_DIRECTORIES", str(tmp_path.parent))


def new_repo(root: Path, gitignore: str) -> Path:
    """A throwaway repository at *root*. The map guard needs one on every command."""
    root.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "init", "-q"], cwd=root, check=True, capture_output=True)
    if gitignore:
        (root / ".gitignore").write_text(gitignore, encoding="utf-8")
    return root


def workspace(tmp_path: Path) -> Path:
    root = new_repo(tmp_path / "repo", "entities.json\n")
    shutil.copy(SAMPLES / "entities.sample.json", root / "entities.json")
    return root


def test_redact_writes_output_and_manifest(tmp_path) -> None:
    root = workspace(tmp_path)
    shutil.copy(SAMPLES / "entities-only.md", root / "in.md")
    code = main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
                 "--out", str(root / "out.md")])
    assert code == 0
    assert "CLIENT_01" in (root / "out.md").read_text(encoding="utf-8")
    manifest = json.loads((root / "out.md.manifest.json").read_text(encoding="utf-8"))
    assert manifest["counts"]["client"] >= 1
    assert "values" not in manifest


def test_redact_halts_writes_triage_and_no_output(tmp_path) -> None:
    root = workspace(tmp_path)
    shutil.copy(SAMPLES / "unmapped-name.md", root / "in.md")
    code = main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
                 "--out", str(root / "out.md")])
    assert code == 2
    assert not (root / "out.md").exists()
    assert not (root / "out.md.manifest.json").exists()
    triage = (root / "out.md.triage.md").read_text(encoding="utf-8")
    assert "John Smith" in triage
    assert "line 3" in triage


def test_the_halt_sentence_is_written_once(tmp_path, capsys) -> None:
    """The triage file and the console must not carry two copies of one sentence.

    Both read the Halt's own message, so the wording lives in errors.py alone
    and the two cannot drift apart.
    """
    root = workspace(tmp_path)
    shutil.copy(SAMPLES / "unmapped-name.md", root / "in.md")
    main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
          "--out", str(root / "out.md")])
    sentence = "1 unclassified candidate(s); nothing was written"
    triage = (root / "out.md.triage.md").read_text(encoding="utf-8")
    assert sentence in triage
    assert sentence in capsys.readouterr().out
    assert sentence not in Path(cli.__file__).read_text(encoding="utf-8")


def test_triage_lists_unknowns_in_line_order(tmp_path) -> None:
    """residual returns all addresses, then all names. A human reads down a page."""
    root = workspace(tmp_path)
    (root / "in.md").write_text(
        "# Meeting note\n\nJohn Smith attended.\nThe office is at 12 Sample Street.\n",
        encoding="utf-8",
    )
    code = main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
                 "--out", str(root / "out.md")])
    assert code == 2
    triage = (root / "out.md.triage.md").read_text(encoding="utf-8")
    assert "line 3" in triage and "line 4" in triage
    assert triage.index("line 3") < triage.index("line 4")


def test_the_cli_never_exposes_a_non_strict_redact() -> None:
    """Task 6 documents a residual limit whose only mitigation is the strict halt.

    That mitigation holds solely while nothing on the operator path redacts
    non-strictly, so the CLI must offer no flag, no variable and no code path
    that reaches redact with strict=False.
    """
    source = Path(cli.__file__).read_text(encoding="utf-8")
    assert "strict=" not in source
    for flag in ("--no-strict", "--strict", "--lenient", "--force"):
        with pytest.raises(SystemExit):
            cli.build_parser().parse_args(
                ["redact", "--in", "a", "--map", "b", "--out", "c", flag]
            )


def test_redact_refuses_an_ungitignored_map(tmp_path) -> None:
    root = new_repo(tmp_path / "repo", "")
    shutil.copy(SAMPLES / "entities.sample.json", root / "entities.json")
    shutil.copy(SAMPLES / "clean.md", root / "in.md")
    code = main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
                 "--out", str(root / "out.md")])
    assert code == 1
    assert not (root / "out.md").exists()


def test_every_command_refuses_a_map_outside_a_work_tree(tmp_path) -> None:
    """The guard runs before the map is read, whichever command was asked for."""
    root = tmp_path / "loose"
    root.mkdir()
    shutil.copy(SAMPLES / "entities.sample.json", root / "entities.json")
    shutil.copy(SAMPLES / "clean.md", root / "in.md")
    source, entity_map = str(root / "in.md"), str(root / "entities.json")
    out = str(root / "out.md")
    for argv in (
        ["redact", "--in", source, "--map", entity_map, "--out", out],
        ["restore", "--in", source, "--map", entity_map, "--out", out],
        ["verify", "--in", source, "--map", entity_map],
    ):
        assert main(argv) == 1, argv[0]
    assert not (root / "out.md").exists()


def test_restore_reverses_the_map(tmp_path) -> None:
    root = workspace(tmp_path)
    original = (SAMPLES / "entities-only.md").read_text(encoding="utf-8")
    shutil.copy(SAMPLES / "entities-only.md", root / "in.md")
    main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
          "--out", str(root / "out.md")])
    code = main(["restore", "--in", str(root / "out.md"), "--map", str(root / "entities.json"),
                 "--out", str(root / "back.md")])
    assert code == 0
    assert (root / "back.md").read_text(encoding="utf-8") == original
    # Bytes, not characters. read_text normalises line endings on the way in
    # and write_text used to rewrite them on the way out, so a round trip on
    # Windows returned the right words in a file that differed on every line.
    assert (root / "back.md").read_bytes() == (root / "in.md").read_bytes()


def test_a_round_trip_preserves_crlf_line_endings(tmp_path) -> None:
    """The boundary replaces identifiers. It must not also rewrite the file."""
    root = workspace(tmp_path)
    source = (SAMPLES / "entities-only.md").read_text(encoding="utf-8")
    (root / "in.md").write_bytes(source.replace("\n", "\r\n").encode("utf-8"))
    main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
          "--out", str(root / "out.md")])
    assert b"\r\n" in (root / "out.md").read_bytes()
    main(["restore", "--in", str(root / "out.md"), "--map", str(root / "entities.json"),
          "--out", str(root / "back.md")])
    assert (root / "back.md").read_bytes() == (root / "in.md").read_bytes()


def test_verify_is_clean_on_redacted_output(tmp_path) -> None:
    root = workspace(tmp_path)
    shutil.copy(SAMPLES / "entities-only.md", root / "in.md")
    main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
          "--out", str(root / "out.md")])
    code = main(["verify", "--in", str(root / "out.md"), "--map", str(root / "entities.json")])
    assert code == 0


def test_verify_reports_findings_with_exit_two(tmp_path, capsys) -> None:
    root = workspace(tmp_path)
    shutil.copy(SAMPLES / "identifiers.md", root / "raw.md")
    code = main(["verify", "--in", str(root / "raw.md"), "--map", str(root / "entities.json")])
    assert code == 2
    assert "tfn" in capsys.readouterr().out


def test_a_missing_input_file_is_exit_one(tmp_path) -> None:
    root = workspace(tmp_path)
    code = main(["redact", "--in", str(root / "absent.md"),
                 "--map", str(root / "entities.json"), "--out", str(root / "out.md")])
    assert code == 1
