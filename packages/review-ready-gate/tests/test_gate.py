from __future__ import annotations

import os
import re
import shutil
from decimal import Decimal
from pathlib import Path

import pytest

from tests.support import EXAMPLES
from reviewready.cli import main
from reviewready.engine import review_pack
from reviewready.errors import GateInputError
from reviewready.models import FINDING_MISSING_ARTEFACT, FINDING_TB_UNBALANCED
from reviewready.report import (
    CHECKOUT_MARKERS,
    PACK_FILE_NAMES,
    _same_directory,
    write_review_pack,
)
from reviewready.viewer import render_review_sheet


def test_bas_ready_pack_is_ready() -> None:
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready")
    assert pack.status == "READY"
    assert pack.findings == ()
    assert pack.period_end == "2026-03-31"
    assert pack.preparer_initials == "AB"


def test_bas_not_ready_missing_gst_and_repeat() -> None:
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-not-ready")
    assert pack.status == "NOT_READY"
    codes = {item.code for item in pack.findings}
    assert FINDING_MISSING_ARTEFACT in codes
    repeats = [item for item in pack.findings if item.repeat]
    assert repeats
    assert any(item.slot == "gst_control_gl" and item.repeat for item in pack.findings)


def test_bas_blocked_unbalanced_trial_balance() -> None:
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-blocked")
    assert pack.status == "BLOCKED"
    assert any(item.code == FINDING_TB_UNBALANCED for item in pack.findings)


def test_month_end_and_year_end_ready() -> None:
    month = review_pack(profile="month_end", pack_dir=EXAMPLES / "month-end-ready")
    year = review_pack(profile="year_end", pack_dir=EXAMPLES / "year-end-ready")
    assert month.status == "READY"
    assert year.status == "READY"


def test_acknowledgement_does_not_change_status() -> None:
    pack = review_pack(
        profile="bas",
        pack_dir=EXAMPLES / "bas-not-ready",
        acknowledgement_path=EXAMPLES / "review_note.json",
    )
    assert pack.status == "NOT_READY"
    assert pack.acknowledgement is not None
    assert pack.acknowledgement.reviewer_initials == "RD"


def test_gst_tieout_break(tmp_path: Path) -> None:
    source = EXAMPLES / "bas-ready"
    pack_dir = tmp_path / "pack"
    pack_dir.mkdir()
    for name in (
        "trial_balance.csv",
        "activity_statement.csv",
        "open_items.csv",
        "self_review.json",
    ):
        (pack_dir / name).write_bytes((source / name).read_bytes())
    (pack_dir / "gst_control_gl.csv").write_text(
        "Date,AccountID,AccountName,Debit,Credit,Description\n"
        "2026-01-15,820,GST Payable,0.00,1.00,Wrong amount\n",
        encoding="utf-8",
    )
    pack = review_pack(profile="bas", pack_dir=pack_dir, tieout_tolerance=Decimal("0.01"))
    assert pack.status == "NOT_READY"
    assert any(item.code == "TIEOUT_BREAK" for item in pack.findings)


def test_cli_ready_and_view(tmp_path: Path) -> None:
    output = tmp_path / "out"
    assert (
        main(
            [
                "gate",
                "--profile",
                "bas",
                "--pack",
                str(EXAMPLES / "bas-ready"),
                "--output",
                str(output),
            ]
        )
        == 0
    )
    assert (output / "readiness-pack.json").is_file()
    assert main(["view", "--pack-dir", str(output)]) == 0
    sheet, digests = render_review_sheet(output)
    assert "**Overall status: READY**" in sheet
    assert "readiness-pack.json" in digests


def test_cli_not_ready_exit_two(tmp_path: Path) -> None:
    assert (
        main(
            [
                "gate",
                "--profile",
                "bas",
                "--pack",
                str(EXAMPLES / "bas-not-ready"),
                "--output",
                str(tmp_path / "out"),
            ]
        )
        == 2
    )


def test_cli_malformed_headers_exit_one(tmp_path: Path) -> None:
    pack_dir = tmp_path / "bad"
    pack_dir.mkdir()
    for name in ("activity_statement.csv", "gst_control_gl.csv", "open_items.csv", "self_review.json"):
        (pack_dir / name).write_bytes((EXAMPLES / "bas-ready" / name).read_bytes())
    (pack_dir / "trial_balance.csv").write_text("Nope\n1\n", encoding="utf-8")
    assert (
        main(
            [
                "gate",
                "--profile",
                "bas",
                "--pack",
                str(pack_dir),
                "--output",
                str(tmp_path / "out"),
            ]
        )
        == 1
    )


def test_unsupported_tie_out(tmp_path: Path) -> None:
    source = EXAMPLES / "year-end-ready"
    pack_dir = tmp_path / "pack"
    pack_dir.mkdir()
    for name in (
        "trial_balance.csv",
        "prior_trial_balance.csv",
        "open_items.csv",
        "self_review.json",
    ):
        (pack_dir / name).write_bytes((source / name).read_bytes())
    (pack_dir / "tie_out_matrix.csv").write_text(
        "StatementLine,StatementAmount,WorkpaperRef,SourceFile,Status\n"
        "Cash,120000.00,,,UNSUPPORTED\n",
        encoding="utf-8",
    )
    pack = review_pack(profile="year_end", pack_dir=pack_dir)
    assert pack.status == "NOT_READY"
    assert any(item.code == "TIEOUT_UNSUPPORTED" for item in pack.findings)


def test_bank_rec_break(tmp_path: Path) -> None:
    source = EXAMPLES / "month-end-ready"
    pack_dir = tmp_path / "pack"
    pack_dir.mkdir()
    for name in (
        "trial_balance.csv",
        "prior_trial_balance.csv",
        "open_items.csv",
        "self_review.json",
    ):
        (pack_dir / name).write_bytes((source / name).read_bytes())
    (pack_dir / "bank_rec.csv").write_text(
        "AccountID,StatementBalance,GLBalance\n100,120000.00,119000.00\n",
        encoding="utf-8",
    )
    pack = review_pack(profile="month_end", pack_dir=pack_dir)
    assert pack.status == "NOT_READY"
    assert any(item.code == "BANK_REC_BREAK" for item in pack.findings)


def test_usage_error_is_exit_one() -> None:
    assert main([]) == 1


# --- a pack never lands inside a checkout ----------------------------------


def _fake_checkout(root: Path, *, git_is_a_file: bool = False, marker: str = ".git") -> Path:
    """Create a directory that looks like a version-control checkout.

    A worktree and a submodule carry a `.git` file holding a gitdir pointer
    rather than a directory, and both are still checkouts, so the guard has to
    read existence rather than directory-ness.
    """
    root.mkdir(parents=True, exist_ok=True)
    if git_is_a_file:
        (root / marker).write_text("gitdir: /elsewhere/.git/worktrees/wt\n", encoding="utf-8")
    else:
        (root / marker).mkdir()
    return root


def _ready_pack():
    return review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready")


def _require_symlinks(tmp_path: Path) -> None:
    """Skip when the host will not create a symbolic link.

    Windows needs Developer Mode or SeCreateSymbolicLinkPrivilege, and this
    package declares no platform restriction, so a contributor running the
    suite there should see a skip rather than an error inside a test whose
    subject is the guard, not the filesystem."""
    probe = tmp_path / "symlink-probe"
    try:
        probe.symlink_to(tmp_path, target_is_directory=True)
    except (OSError, NotImplementedError) as exc:  # pragma: no cover - POSIX CI
        pytest.skip(f"symbolic links unavailable here: {exc}")
    probe.unlink()


def test_writer_refuses_an_output_inside_a_checkout(tmp_path: Path) -> None:
    """A readiness pack names a client's file, its workpaper references and
    every finding standing between it and manager review. Inside a checkout it
    is one `git add -A` away from a history every clone copies, and a
    .gitignore entry is a convention the next commit can waive."""
    checkout = _fake_checkout(tmp_path / "firm-repo")

    with pytest.raises(GateInputError, match="inside the version-control checkout"):
        write_review_pack(_ready_pack(), checkout / "packs" / "march")


def test_a_refused_output_leaves_nothing_behind(tmp_path: Path) -> None:
    """The guard runs before mkdir, so a refused run does not create the tree it
    was told to write into and then abandon it."""
    checkout = _fake_checkout(tmp_path / "firm-repo")

    with pytest.raises(GateInputError):
        write_review_pack(_ready_pack(), checkout / "packs" / "march")

    assert not (checkout / "packs").exists()


@pytest.mark.parametrize("git_is_a_file", [False, True])
def test_a_worktree_pointer_counts_as_a_checkout(tmp_path: Path, git_is_a_file: bool) -> None:
    checkout = _fake_checkout(tmp_path / "firm-repo", git_is_a_file=git_is_a_file)

    with pytest.raises(GateInputError, match=str(checkout)):
        write_review_pack(_ready_pack(), checkout / "packs")


def test_the_checkout_root_itself_is_refused(tmp_path: Path) -> None:
    """`--output .` from a repository root is the same mistake as any nested
    path, so the directory itself counts, not only its parents."""
    checkout = _fake_checkout(tmp_path / "firm-repo")

    with pytest.raises(GateInputError, match="inside the version-control checkout"):
        write_review_pack(_ready_pack(), checkout)


def test_a_relative_path_climbing_into_a_checkout_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The path is resolved first, so `..` segments and a working directory
    cannot walk a pack back into a repository the literal argument never named."""
    checkout = _fake_checkout(tmp_path / "firm-repo")
    outside = tmp_path / "elsewhere"
    outside.mkdir()
    monkeypatch.chdir(outside)

    with pytest.raises(GateInputError, match="inside the version-control checkout"):
        write_review_pack(_ready_pack(), Path("..") / "firm-repo" / "packs")


def test_an_output_outside_any_checkout_still_writes(tmp_path: Path) -> None:
    """The guard must not refuse the ordinary case: an access-controlled
    directory that is not under version control at all."""
    _fake_checkout(tmp_path / "firm-repo")
    outputs = write_review_pack(_ready_pack(), tmp_path / "review-data" / "march")

    assert sorted(path.name for path in outputs.values()) == sorted(PACK_FILE_NAMES)


def test_a_symlink_swapped_after_the_check_cannot_redirect_the_pack(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The guard's decision has to bind to the destination it approved.

    `resolve` follows a path's symlinks once; every later `mkdir` and
    `write_text` follows them again. The writer takes the resolved directory
    back from the guard and builds every destination from it, so the swap below
    changes nothing.
    """
    _require_symlinks(tmp_path)
    safe = tmp_path / "review-data"
    safe.mkdir()
    checkout = _fake_checkout(tmp_path / "firm-repo")
    link = tmp_path / "link"
    link.symlink_to(safe, target_is_directory=True)

    real_mkdir = Path.mkdir

    def swap_then_mkdir(self: Path, *args: object, **kwargs: object) -> None:
        link.unlink()
        link.symlink_to(checkout, target_is_directory=True)
        return real_mkdir(self, *args, **kwargs)  # type: ignore[arg-type]

    monkeypatch.setattr(Path, "mkdir", swap_then_mkdir)
    outputs = write_review_pack(_ready_pack(), link / "march")

    assert (safe / "march" / "readiness-pack.json").exists()
    assert not list(checkout.glob("march"))
    for path in outputs.values():
        assert safe in path.parents


def test_the_cli_refuses_before_it_reads_a_workpaper_pack(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """A reviewer who mistyped --output should hear about it before the run
    opens a client's workpapers, and the exit code is the documented 1 for an
    invalid command."""
    checkout = _fake_checkout(tmp_path / "firm-repo")

    code = main([
        "gate",
        "--profile", "bas",
        "--pack", str(tmp_path / "does-not-exist"),
        "--output", str(checkout / "packs"),
    ])

    assert code == 1
    error = capsys.readouterr().err
    assert "output error" in error
    assert "inside the version-control checkout" in error
    # The input error never fires: the run stopped before reading anything.
    assert "input error" not in error


@pytest.mark.parametrize("marker", CHECKOUT_MARKERS)
def test_every_checkout_marker_is_refused(tmp_path: Path, marker: str) -> None:
    """The harm is the pack going under version control, and Mercurial,
    Subversion and Bazaar copy a committed pack to every clone exactly as Git
    does. A firm on one of them is the one least likely to be told the tool
    assumed the other."""
    checkout = _fake_checkout(tmp_path / "firm-repo", marker=marker)

    with pytest.raises(GateInputError, match=f"which holds {re.escape(marker)}"):
        write_review_pack(_ready_pack(), checkout / "packs" / "march")


def test_a_work_tree_named_only_by_the_environment_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A tracked work tree need not carry a marker to walk up to.

    Git can keep its metadata elsewhere, with GIT_DIR and GIT_WORK_TREE. The
    work tree then holds tracked files and no `.git` at all, so the ancestor
    scan finds nothing and would approve a pack written among them. Where the
    environment names that tree, the guard can see it and refuses.
    """
    work_tree = tmp_path / "client-files"
    work_tree.mkdir()
    monkeypatch.setenv("GIT_WORK_TREE", str(work_tree))
    monkeypatch.setenv("GIT_DIR", str(tmp_path / "metadata.git"))

    assert not any((work_tree / marker).exists() for marker in CHECKOUT_MARKERS)
    with pytest.raises(GateInputError, match="GIT_WORK_TREE"):
        write_review_pack(_ready_pack(), work_tree / "packs" / "march")


def test_two_spellings_of_one_directory_are_recognised_as_one(
    tmp_path: Path,
) -> None:
    """The comparison the work-tree check rests on answers by identity.

    A case alias needs a case-insensitive filesystem, which the Linux runners
    are not, so the test above skips there and this one carries the mechanism.
    A symbolic link is the same shape of question, two paths and one inode, and
    every supported host can make one: `Path` equality says they differ, and
    the guard's comparison says they do not.
    """
    _require_symlinks(tmp_path)
    real = tmp_path / "client-files"
    real.mkdir()
    alias = tmp_path / "alias"
    alias.symlink_to(real, target_is_directory=True)

    assert alias != real
    assert _same_directory(alias, real)
    assert not _same_directory(tmp_path / "elsewhere", real)


def test_a_differently_cased_work_tree_is_still_the_same_work_tree(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Comparing spelling would let a case alias walk past the guard.

    macOS and Windows accept both spellings of a directory and hand back
    whichever the caller used, so a work tree named `client-files` and an
    output written under `CLIENT-FILES` are one directory that plain `Path`
    equality calls two. Linux runners are case-sensitive, where the two really
    are separate directories and there is nothing to test, so this skips there
    rather than asserting something the host cannot show.
    """
    work_tree = tmp_path / "client-files"
    work_tree.mkdir()
    alias = tmp_path / "CLIENT-FILES"
    if not alias.exists():  # pragma: no cover - POSIX CI
        pytest.skip("this filesystem is case-sensitive, so there is no alias")
    monkeypatch.setenv("GIT_WORK_TREE", str(work_tree))

    with pytest.raises(GateInputError, match="GIT_WORK_TREE"):
        write_review_pack(_ready_pack(), alias / "packs" / "march")


def test_a_work_tree_elsewhere_does_not_refuse_an_unrelated_output(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The environment check refuses what is inside that tree, not everything."""
    monkeypatch.setenv("GIT_WORK_TREE", str(tmp_path / "client-files"))
    outside = tmp_path / "review-ready-demo" / "march"

    outputs = write_review_pack(_ready_pack(), outside)

    assert sorted(path.name for path in outputs.values()) == sorted(PACK_FILE_NAMES)


def test_a_symlink_loop_in_the_output_path_is_refused(tmp_path: Path) -> None:
    """One refusal on every supported interpreter, from the guard.

    `Path.resolve` raises RuntimeError, not OSError, for a symlink loop before
    Python 3.13, which left the CLI's handlers untouched and printed a
    traceback. From 3.13 resolve hands back the unresolved path instead, and
    the loop surfaced two layers later in mkdir. The guard stats the resolved
    destination so both end here, with the same error."""
    _require_symlinks(tmp_path)
    looped = tmp_path / "loop"
    other = tmp_path / "other"
    looped.symlink_to(other)
    other.symlink_to(looped)

    with pytest.raises(GateInputError, match="cannot be examined"):
        write_review_pack(_ready_pack(), looped / "march")


def test_an_uninspectable_marker_is_refused_rather_than_assumed_absent(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A marker this run cannot read is not a marker that is not there.

    `os.lstat` is patched, not `Path.exists`, because the guard deliberately
    stopped asking exists(): up to 3.13 it swallowed a symlink loop and
    propagated a permission error, and from 3.14 it answers False for every
    OSError, which would let an unreadable `.git` read as an absent one and
    approve an output inside the checkout it could not see. Patching lstat
    tests the call the guard actually makes, on every interpreter.

    Patched rather than chmod-ed because the suite may run as a user that
    bypasses directory permissions, which would turn this into a test of
    nothing."""
    real_lstat = os.lstat

    def refuse_to_stat(path, *args: object, **kwargs: object):
        if Path(path).name in CHECKOUT_MARKERS:
            raise PermissionError(13, "Permission denied")
        return real_lstat(path, *args, **kwargs)

    monkeypatch.setattr(os, "lstat", refuse_to_stat)

    with pytest.raises(GateInputError, match="cannot be examined"):
        write_review_pack(_ready_pack(), tmp_path / "outside" / "march")


def test_a_marker_that_is_a_dangling_symlink_still_counts(tmp_path: Path) -> None:
    """lstat does not follow the entry, so a `.git` symlink is evidence of a
    checkout whether or not its target is currently reachable. Following it
    would let a broken pointer read as no checkout at all."""
    _require_symlinks(tmp_path)
    checkout = tmp_path / "firm-repo"
    checkout.mkdir()
    (checkout / ".git").symlink_to(tmp_path / "nowhere")

    with pytest.raises(GateInputError, match="inside the version-control checkout"):
        write_review_pack(_ready_pack(), checkout / "packs" / "march")


def test_the_cli_reports_an_uninspectable_output_as_exit_one(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """The documented contract is exit 1 for an --output that cannot be
    written, so a path the guard cannot examine must not escape as a
    traceback."""
    _require_symlinks(tmp_path)
    looped = tmp_path / "loop"
    other = tmp_path / "other"
    looped.symlink_to(other)
    other.symlink_to(looped)

    code = main([
        "gate",
        "--profile", "bas",
        "--pack", str(EXAMPLES / "bas-ready"),
        "--output", str(looped / "march"),
    ])

    assert code == 1
    assert "output error" in capsys.readouterr().err


def test_view_still_opens_a_pack_that_is_inside_a_checkout(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """The refusal is on writing. A firm that already has packs under version
    control must still be able to read them, or the guard would take evidence
    out of its hands."""
    checkout = _fake_checkout(tmp_path / "firm-repo")
    pack_dir = tmp_path / "outside" / "march"
    write_review_pack(_ready_pack(), pack_dir)
    moved = checkout / "archive"
    shutil.copytree(pack_dir, moved)

    assert main(["view", "--pack-dir", str(moved)]) == 0
    assert "Overall status: READY" in capsys.readouterr().out
