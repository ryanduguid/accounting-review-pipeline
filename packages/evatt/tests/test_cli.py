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


def workspace(
    tmp_path: Path,
    gitignore: str = "entities.json\n*.triage.md\n",
    name: str = "entities.json",
) -> Path:
    """A repository holding a gitignored copy of the sample map.

    The collision tests need the map under a name of their own choosing, so the
    ignore rule and the file name are both adjustable. Everything else uses the
    defaults and reads the same as it always did.

    The default rules are the two the package's own .gitignore ships and the CI
    demo writes. ``*.triage.md`` is not decoration: a halt asks git about the
    triage path before it writes it, so a workspace without that rule refuses
    to halt at all. ``test_a_halt_refuses_a_committable_triage_path`` is the
    test that pins the refusal, and it builds its own repository without it.
    """
    root = new_repo(tmp_path / "repo", gitignore)
    shutil.copy(SAMPLES / "entities.sample.json", root / name)
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


def test_redact_refuses_an_ungitignored_map(tmp_path, capsys) -> None:
    """The branch is asserted, not just the exit code.

    This test and the work-tree one below both exit 1 on the same guard, so
    without matching the message the two could swap which branch they exercise
    and both would still pass.
    """
    root = new_repo(tmp_path / "repo", "")
    shutil.copy(SAMPLES / "entities.sample.json", root / "entities.json")
    shutil.copy(SAMPLES / "clean.md", root / "in.md")
    code = main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
                 "--out", str(root / "out.md")])
    assert code == 1
    assert not (root / "out.md").exists()
    assert "git does not ignore it" in capsys.readouterr().err


def test_every_command_refuses_a_map_outside_a_work_tree(tmp_path, capsys) -> None:
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
        assert "it is not inside a git work tree" in capsys.readouterr().err, argv[0]
    assert not (root / "out.md").exists()


def test_an_error_goes_to_stderr_and_not_to_stdout(tmp_path, capsys) -> None:
    """argparse writes its own failures there, so these have to match."""
    root = workspace(tmp_path)
    assert main(["redact", "--in", str(root / "absent.md"),
                 "--map", str(root / "entities.json"), "--out", str(root / "out.md")]) == 1
    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err.startswith("error: ")


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


# A wrapped identifier is the case the round-trip test above cannot reach:
# entities-only.md holds no identifier at all, so CRLF parity was never what
# that test proved, only that the bytes came back.
WRAPPED = "Engagement file.\n\nTFN: 123 456\n782 was quoted.\n"


def test_crlf_and_lf_copies_of_one_document_redact_identically(tmp_path) -> None:
    """A CRLF break inside an identifier used to defeat every digit pattern.

    Each pattern separates groups with ``[\\s-]?``, exactly one character, and a
    CRLF pair is two. The LF copy produced "TFN: TFN_01 here." and a manifest
    counting one tfn; the CRLF copy wrote the live tax file number straight
    through with an empty manifest, and verify then called it clean. Detection
    must not depend on how the operator's editor saved the file.
    """
    root = workspace(tmp_path)
    entity_map = str(root / "entities.json")
    outputs = {}
    for label, payload in (("lf", WRAPPED), ("crlf", WRAPPED.replace("\n", "\r\n"))):
        source = root / f"{label}.md"
        source.write_bytes(payload.encode("utf-8"))
        out = root / f"{label}.out.md"
        assert main(["redact", "--in", str(source), "--map", entity_map,
                     "--out", str(out)]) == 0, label
        text = out.read_bytes().decode("utf-8").replace("\r\n", "\n")
        counts = json.loads(
            (root / f"{label}.out.md.manifest.json").read_text(encoding="utf-8")
        )["counts"]
        outputs[label] = (text, counts)
        assert "123 456" not in text, label
        assert "TFN_01" in text, label
    assert outputs["lf"] == outputs["crlf"]
    assert outputs["lf"][1]["tfn"] == 1
    # The ending the source carried is the ending the output gets back.
    assert b"\r\n" in (root / "crlf.out.md").read_bytes()
    assert b"\r" not in (root / "lf.out.md").read_bytes()


def test_verify_sees_a_wrapped_identifier_in_a_crlf_file(tmp_path) -> None:
    """The operator's last check cleared a live TFN, which is the worse half."""
    root = workspace(tmp_path)
    (root / "in.md").write_bytes(WRAPPED.replace("\n", "\r\n").encode("utf-8"))
    code = main(["verify", "--in", str(root / "in.md"), "--map", str(root / "entities.json")])
    assert code == 2


def test_redact_refuses_to_overwrite_the_map_or_the_input(tmp_path, capsys) -> None:
    """One typo destroyed the only copy of the key, and it exited 0 doing it.

    Structured identifiers are replaced one way, so every document already
    redacted against that map becomes unrestorable. The two paths ``--out``
    derives are covered too: neither is spelt on the command line, so neither
    is a collision the operator can see coming.
    """
    root = workspace(tmp_path, gitignore="*.json\n", name="key.json")
    entity_map, source = root / "key.json", root / "in.md"
    shutil.copy(SAMPLES / "entities-only.md", source)
    before = entity_map.read_bytes()

    for out, expected in (
        (entity_map, "--out is the entity map"),
        (source, "--out is the input"),
    ):
        assert main(["redact", "--in", str(source), "--map", str(entity_map),
                     "--out", str(out)]) == 1, out
        assert expected in capsys.readouterr().err, out

    # The manifest --out derives lands on the map.
    on_manifest = workspace(tmp_path / "b", gitignore="*.json\n", name="out.md.manifest.json")
    shutil.copy(SAMPLES / "entities-only.md", on_manifest / "in.md")
    assert main(["redact", "--in", str(on_manifest / "in.md"),
                 "--map", str(on_manifest / "out.md.manifest.json"),
                 "--out", str(on_manifest / "out.md")]) == 1
    assert "the manifest path --out derives is the entity map" in capsys.readouterr().err

    # The triage path --out derives lands on the input.
    on_triage = workspace(tmp_path / "c")
    shutil.copy(SAMPLES / "entities-only.md", on_triage / "out.md.triage.md")
    assert main(["redact", "--in", str(on_triage / "out.md.triage.md"),
                 "--map", str(on_triage / "entities.json"),
                 "--out", str(on_triage / "out.md")]) == 1
    assert "the triage path --out derives is the input" in capsys.readouterr().err

    assert entity_map.read_bytes() == before
    original = (SAMPLES / "entities-only.md").read_text(encoding="utf-8")
    assert source.read_text(encoding="utf-8") == original


def test_restore_refuses_to_overwrite_the_map_or_the_input(tmp_path, capsys) -> None:
    root = workspace(tmp_path, gitignore="*.json\n", name="key.json")
    entity_map, source = root / "key.json", root / "in.md"
    shutil.copy(SAMPLES / "entities-only.md", source)
    before = entity_map.read_bytes()
    for out, expected in (
        (entity_map, "--out is the entity map"),
        (source, "--out is the input"),
    ):
        assert main(["restore", "--in", str(source), "--map", str(entity_map),
                     "--out", str(out)]) == 1, out
        assert expected in capsys.readouterr().err, out
    assert entity_map.read_bytes() == before


LEAKY = (
    "# Prior pack\n\n"
    "CLIENT_01 was quoted from a prior pack; Sample Holdings Pty Ltd TFN 123 456 782\n"
    "and a.person@example.com, at 12 Sample Street.\n"
)


def test_no_triage_context_carries_a_value_redaction_replaced(tmp_path) -> None:
    """The triage file held both forms of the same line, and one was the raw one.

    ``residual`` quoted the redacted text while the carried-placeholder sweep
    quoted the input, so a full tax file number, an email address and a mapped
    client name landed in plaintext in the directory the operator sends from.
    """
    root = workspace(tmp_path)
    (root / "in.md").write_text(LEAKY, encoding="utf-8")
    code = main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
                 "--out", str(root / "out.md")])
    assert code == 2
    triage = (root / "out.md.triage.md").read_text(encoding="utf-8")
    contexts = [line for line in triage.splitlines() if line.startswith("  > ")]
    assert contexts
    for replaced in ("Sample Holdings Pty Ltd", "123 456 782", "a.person@example.com"):
        assert not any(replaced in context for context in contexts), replaced
        assert replaced not in triage, replaced
    # The placeholder is still reported, quoted against its redacted line.
    assert "**CLIENT_01** (placeholder" in triage
    assert any("CLIENT_01 was quoted" in context for context in contexts)


def test_the_triage_file_is_gitignored() -> None:
    """The package rule has to cover the name the CLI actually derives."""
    gitignore = Path(cli.__file__).resolve().parents[1] / ".gitignore"
    lines = [line.strip() for line in gitignore.read_text(encoding="utf-8").splitlines()]
    assert "*.triage.md" in lines


def test_an_output_that_cannot_be_written_is_exit_one(tmp_path, capsys) -> None:
    """The try covered the reads only, so this raised a traceback."""
    root = workspace(tmp_path)
    shutil.copy(SAMPLES / "entities-only.md", root / "in.md")
    (root / "blocked").write_text("not a directory", encoding="utf-8")
    code = main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
                 "--out", str(root / "blocked" / "out.md")])
    assert code == 1
    assert capsys.readouterr().err.startswith("error: ")


def test_a_manifest_that_cannot_be_written_takes_the_output_with_it(tmp_path, capsys) -> None:
    """The manifest fails after out.md is already on disk.

    A sanitised document with no manifest is one nobody can say what was
    replaced in, so it does not survive the failure either.
    """
    root = workspace(tmp_path)
    shutil.copy(SAMPLES / "entities-only.md", root / "in.md")
    (root / "out.md.manifest.json").mkdir()
    code = main(["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
                 "--out", str(root / "out.md")])
    assert code == 1
    assert not (root / "out.md").exists()
    assert capsys.readouterr().err.startswith("error: ")


def test_a_halt_removes_an_earlier_runs_output(tmp_path) -> None:
    """Nothing written has to be true of the directory, not just of this run.

    Run one succeeds. The input then gains an unmapped name and run two halts,
    leaving run one's out.md beside the new triage file, which is the file an
    operator sends.
    """
    root = workspace(tmp_path)
    out, manifest = root / "out.md", root / "out.md.manifest.json"
    argv = ["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
            "--out", str(out)]
    shutil.copy(SAMPLES / "entities-only.md", root / "in.md")
    assert main(argv) == 0
    assert out.exists() and manifest.exists()

    shutil.copy(SAMPLES / "unmapped-name.md", root / "in.md")
    assert main(argv) == 2
    assert not out.exists()
    assert not manifest.exists()
    assert (root / "out.md.triage.md").exists()


def test_a_usage_error_is_exit_one_not_two(tmp_path, capsys) -> None:
    """The house convention reads 2 as halted or findings.

    argparse exits 2 for a typo, so a wrapper could not tell a mistyped
    command from a document that needs triage.
    """
    for argv in ([], ["nosuchcommand"], ["redact", "--in", "a", "--map", "b"]):
        assert main(argv) == 1, argv
        assert capsys.readouterr().err, argv
    # --help and --version are argparse doing what it was asked, and still exit 0.
    for argv in (["--help"], ["--version"]):
        with pytest.raises(SystemExit) as caught:
            main(argv)
        assert caught.value.code == 0, argv


def test_a_halt_refuses_a_committable_triage_path(tmp_path, capsys) -> None:
    """The triage file is guarded before a byte reaches it, the way the map's .tmp is.

    It quotes whole residual lines about a real document at a path the operator
    never typed, because --out derives it. A run that cannot write it safely
    writes nothing at all, and the stale output goes anyway.
    """
    root = workspace(tmp_path, gitignore="entities.json\n")
    shutil.copy(SAMPLES / "entities-only.md", root / "in.md")
    argv = ["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
            "--out", str(root / "out.md")]
    assert main(argv) == 0
    capsys.readouterr()

    shutil.copy(SAMPLES / "unmapped-name.md", root / "in.md")
    assert main(argv) == 1
    error = capsys.readouterr().err
    assert "the triage file must never be committed" in error
    assert "git does not ignore it" in error
    assert not (root / "out.md.triage.md").exists()
    assert not (root / "out.md").exists()
    assert not (root / "out.md.manifest.json").exists()


def test_a_clean_run_removes_an_earlier_runs_triage_file(tmp_path) -> None:
    """The other direction of the same argument the halt path already accepted.

    Run one halts and writes a triage file naming a real person. Run two
    succeeds. Without this the operator sees exit 0 and a sanitised document
    with that plaintext worklist sitting in the directory they send from.
    """
    root = workspace(tmp_path)
    triage = root / "out.md.triage.md"
    argv = ["redact", "--in", str(root / "in.md"), "--map", str(root / "entities.json"),
            "--out", str(root / "out.md")]
    shutil.copy(SAMPLES / "unmapped-name.md", root / "in.md")
    assert main(argv) == 2
    assert "John Smith" in triage.read_text(encoding="utf-8")

    shutil.copy(SAMPLES / "entities-only.md", root / "in.md")
    assert main(argv) == 0
    assert (root / "out.md").exists()
    assert not triage.exists()


def test_a_triage_value_holding_a_line_break_keeps_its_bold_span(tmp_path) -> None:
    """NAME spans a newline, so a candidate can carry one, and markdown cannot.

    The raw value broke the bold span open across two lines and left the
    worklist unreadable at exactly the entry that needed reading.
    """
    root = workspace(tmp_path)
    (root / "in.md").write_text("# meeting\n\nJohn\nSmith attended.\n", encoding="utf-8")
    assert main(["redact", "--in", str(root / "in.md"),
                 "--map", str(root / "entities.json"),
                 "--out", str(root / "out.md")]) == 2
    triage = (root / "out.md.triage.md").read_text(encoding="utf-8")
    assert "**John Smith** (name, line 3)" in triage
    # The candidate really did carry the break, so the collapse is what fixed it.
    assert "John\nSmith" not in triage


CASE_MATRIX = (
    ("Jane Roe", "PERSON_01"),
    ("JANE ROE", "PERSON_01"),
    ("Jane  Roe", "PERSON_01"),
    ("Jane\nRoe", "PERSON_01"),
    ("jane roe", "PERSON_01"),
    ("Jane roe", "PERSON_01"),
    ("sample holdings pty ltd", "CLIENT_01"),
)


def test_no_case_or_wrapping_of_a_mapped_name_survives_redact_and_verify(tmp_path) -> None:
    """The finding-1 matrix, end to end through the commands an operator runs.

    Three of these rows left the name in the output with an empty manifest, and
    verify then exited 0 on the leaked file. The other four halted, which was
    safe but still wrong: the triage file asked the operator to classify a name
    the map already held, and assign would have minted a second placeholder.
    """
    root = workspace(tmp_path)
    for index, (form, placeholder) in enumerate(CASE_MATRIX):
        source, out = root / f"in{index}.md", root / f"out{index}.md"
        source.write_text("client: %s\n" % form, encoding="utf-8")
        assert main(["redact", "--in", str(source), "--map", str(root / "entities.json"),
                     "--out", str(out)]) == 0, form
        written = out.read_text(encoding="utf-8")
        assert written == "client: %s\n" % placeholder, form
        for token in form.split():
            assert token.casefold() not in written.casefold(), form
        manifest = json.loads((root / f"out{index}.md.manifest.json").read_text(encoding="utf-8"))
        assert sum(manifest["counts"].values()) == 1, form
        assert main(["verify", "--in", str(out), "--map", str(root / "entities.json")]) == 0, form


def test_verify_exits_two_on_a_mapped_name_left_in_lower_case(tmp_path) -> None:
    """The half of finding 1 verify owns: it must not agree with the leak."""
    root = workspace(tmp_path)
    (root / "leaked.md").write_text("client: sample holdings pty ltd\n", encoding="utf-8")
    assert main(["verify", "--in", str(root / "leaked.md"),
                 "--map", str(root / "entities.json")]) == 2
