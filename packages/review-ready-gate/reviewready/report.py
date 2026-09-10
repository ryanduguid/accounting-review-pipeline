from __future__ import annotations

import csv
import io
import json
import os
import uuid
from pathlib import Path

from .engine import ReadinessPack
from .errors import GateInputError
from .models import Finding

REVIEW_BOUNDARY = (
    "This pack is a review aid. It does not approve a file, lodge a return, "
    "or replace professional judgement."
)

PACK_FILE_NAMES = ("readiness-pack.json", "readiness-summary.md", "findings.csv")


def _money(value) -> str:
    if value is None:
        return ""
    as_tuple = getattr(value, "as_tuple", None)
    places = 2
    if as_tuple is not None:
        exponent = as_tuple().exponent
        if isinstance(exponent, int):
            places = max(2, -exponent)
    return f"{value:.{places}f}"


def _csv_safe(value: str) -> str:
    stripped = value.lstrip()
    if stripped.startswith(("=", "+", "-", "@")):
        return "'" + value
    return value


def _md_cell(value: str) -> str:
    # Backslash first, or the escapes below would be escaped again. Asterisk and
    # backtick are structural in the summary the viewer verifies: without them a
    # preparer-supplied cell can forge a status line or an evidence digest line.
    return (
        " ".join(value.split())
        .replace("\\", "\\\\")
        .replace("|", "\\|")
        .replace("*", "\\*")
        .replace("`", "\\`")
    )


def _md_note_lines(comment: str) -> list[str]:
    if "\n" not in comment and "\r" not in comment:
        return [f"- Comment: {_md_cell(comment)}"]
    lines = ["- Comment:"]
    for raw in comment.splitlines():
        line = _md_cell(raw)
        if line.startswith("#"):
            line = "\\" + line
        lines.append(f"  > {line}".rstrip())
    return lines


def _finding_dict(item: Finding) -> dict[str, str]:
    return {
        "code": item.code,
        "status": item.status,
        "slot": item.slot,
        "repeat": "true" if item.repeat else "false",
        "reason": item.reason,
        "reviewer_action": item.reviewer_action,
    }


def _as_json(pack: ReadinessPack) -> dict:
    acknowledgement = None
    if pack.acknowledgement is not None:
        acknowledgement = {
            "reviewer_initials": pack.acknowledgement.reviewer_initials,
            "reviewed_on": pack.acknowledgement.reviewed_on.isoformat(),
            "comment": pack.acknowledgement.comment,
            "effect": (
                "Acknowledgement is evidence of human review only; it does not "
                "change readiness or approve a file."
            ),
        }
    return {
        "acknowledgement": acknowledgement,
        "engagement_type": pack.engagement_type,
        "findings": [_finding_dict(item) for item in pack.findings],
        "overall_status": pack.status,
        "period_end": pack.period_end,
        "preparer_initials": pack.preparer_initials,
        "review_boundary": REVIEW_BOUNDARY,
        "source_sha256": {
            evidence.slot: {"filename": evidence.filename, "sha256": evidence.sha256}
            for evidence in pack.source_evidence
        },
        "thresholds": {"tieout_tolerance": _money(pack.tieout_tolerance)},
    }


_ABSENT = "n/a"


def _as_markdown(pack: ReadinessPack) -> str:
    blocked = sum(finding.status == "BLOCKED" for finding in pack.findings)
    not_ready = sum(finding.status == "NOT_READY" for finding in pack.findings)
    repeats = sum(finding.repeat for finding in pack.findings)
    lines = [
        "# Review-Ready Pack",
        "",
        f"**Overall status: {pack.status}**",
        "",
        REVIEW_BOUNDARY,
        "",
        "## Scope",
        "",
        f"- Engagement type: {pack.engagement_type}",
        f"- Period end: {pack.period_end or _ABSENT}",
        f"- Preparer initials: {_md_cell(pack.preparer_initials) if pack.preparer_initials else _ABSENT}",
        f"- Tie-out tolerance: ${_money(pack.tieout_tolerance)}",
        f"- Findings: {len(pack.findings)} total; {blocked} blocked; {not_ready} not ready; {repeats} repeats.",
        "",
        "## Source evidence",
        "",
    ]
    if not pack.source_evidence:
        lines.append("No source files were present in the pack directory.")
    else:
        for evidence in pack.source_evidence:
            lines.append(f"- `{evidence.slot}` (`{evidence.filename}`): `{evidence.sha256}`")
    lines += ["", "## Findings", ""]
    if not pack.findings:
        lines.append(
            "No readiness findings were raised. A human must still decide whether the file is acceptable."
        )
    else:
        lines += [
            "| Status | Code | Slot | Repeat | Reason |",
            "| --- | --- | --- | --- | --- |",
        ]
        for finding in pack.findings:
            lines.append(
                f"| {finding.status} | {finding.code} | {_md_cell(finding.slot)} | "
                f"{'yes' if finding.repeat else 'no'} | {_md_cell(finding.reason)} |"
            )
    lines += ["", "## Human acknowledgement", ""]
    if pack.acknowledgement is None:
        lines.append("No reviewer acknowledgement was supplied. This does not create or imply an approval.")
    else:
        lines += [
            f"- Reviewer initials: {_md_cell(pack.acknowledgement.reviewer_initials)}",
            f"- Reviewed on: {pack.acknowledgement.reviewed_on.isoformat()}",
            *_md_note_lines(pack.acknowledgement.comment),
            "- Effect: acknowledgement records a human action only; it does not change readiness or approve a file.",
        ]
    lines.append("")
    return "\n".join(lines)


def _as_csv(pack: ReadinessPack) -> str:
    fields = ["code", "status", "slot", "repeat", "reason", "reviewer_action"]
    buffer = io.StringIO()
    writer = csv.DictWriter(buffer, fieldnames=fields)
    writer.writeheader()
    for item in pack.findings:
        row = _finding_dict(item)
        for field in ("slot", "reason", "reviewer_action"):
            row[field] = _csv_safe(row[field])
        writer.writerow(row)
    return buffer.getvalue()


def _remove_quietly(path: Path) -> None:
    try:
        path.unlink()
    except OSError:
        pass


def _restore_quietly(parked: Path, destination: Path) -> None:
    try:
        os.replace(parked, destination)
    except OSError:
        pass


def _sibling_partial(destination: Path) -> Path:
    while True:
        candidate = destination.with_name(f"{destination.name}.{uuid.uuid4().hex[:12]}.partial")
        try:
            with candidate.open("x", encoding="utf-8"):
                pass
        except FileExistsError:  # pragma: no cover
            continue
        return candidate


def _swap_into_place(staged_path: Path, destination: Path) -> Path | None:
    parked: Path | None = None
    if destination.is_file():
        parked = _sibling_partial(destination)
        try:
            os.replace(destination, parked)
        except OSError:
            _remove_quietly(parked)
            raise
    try:
        os.replace(staged_path, destination)
    except OSError:
        if parked is not None:
            _restore_quietly(parked, destination)
        raise
    return parked


CHECKOUT_MARKERS = (".git", ".hg", ".svn", ".bzr")
"""The metadata entries whose presence marks a directory as a checkout root.

Git is what this repository is kept in, but the reason to refuse is that the
pack lands under version control, and Mercurial, Subversion and Bazaar copy a
committed pack to every clone exactly as Git does. A firm on one of them is
the firm least likely to be told this tool assumed the other.
"""


def _reject_unreachable(directory: Path) -> None:
    """Raise OSError if a resolved destination cannot be reached at all.

    ``Path.resolve`` reports a symbolic-link loop as RuntimeError before Python
    3.13 and simply hands back the unresolved path from 3.13, so on the newer
    interpreters the loop shows up only when something stats the path. Stating
    it here keeps every supported version refusing in the same place, instead
    of one of them walking a path it never resolved and failing later in mkdir.

    A destination that is not there yet is the ordinary case: the writer
    creates it.
    """
    for candidate in (directory, *directory.parents):
        try:
            candidate.stat()
        except FileNotFoundError:
            # A missing child can hide an unreachable parent on Windows.
            continue
        return


def _marker_present(candidate: Path, marker: str) -> bool:
    """Is the marker there? Raise OSError when that cannot be determined.

    ``os.lstat`` rather than ``Path.exists``, because exists() decides for
    itself which failures mean "no" and that decision has moved twice: up to
    3.13 it swallowed a symlink loop and propagated a permission error, and
    from 3.14 it returns False for every OSError. On 3.14 an unreadable marker
    would therefore read as an absent one and the guard would approve an output
    inside the very checkout it could not see. This package sets no upper bound
    on the interpreter, so that is a version it will meet.

    lstat answers only the question asked and leaves the caller to judge the
    failures. It also does not follow the entry, so a ``.git`` symlink counts
    as the checkout it names rather than as whatever it points at.
    """
    try:
        os.lstat(candidate / marker)
    except (FileNotFoundError, NotADirectoryError):
        # Genuinely not there, or a path component that cannot hold one.
        return False
    return True


def _configured_work_tree() -> Path | None:
    """The work tree this process's environment points Git at, if any.

    Git can keep its metadata away from the files it tracks, through GIT_DIR
    and GIT_WORK_TREE, --git-dir and --work-tree, or core.worktree. A work tree
    arranged that way holds no marker at all, so the ancestor scan walks
    straight past it and would approve a pack written among tracked files.

    Only the environment is readable from here. A work tree chosen by a flag on
    someone else's git invocation, or by core.worktree in a repository this
    process never opens, cannot be discovered, so this closes the case this
    process can see rather than the whole class. That is the honest limit of
    the guard: it is a backstop for a location the firm chose, not a proof that
    a directory is untracked.
    """
    value = os.environ.get("GIT_WORK_TREE")
    if not value:
        return None
    return Path(value).resolve()


def _same_directory(candidate: Path, work_tree: Path) -> bool:
    """Do these two paths name one directory, whatever they are spelled?

    ``Path`` equality compares text. ``Path.resolve`` normalises separators,
    ``..`` and symbolic links, but it does not normalise case, and it hands
    back the spelling it was given: macOS and Windows will both accept
    ``/Users/x/Repo`` and ``/Users/x/repo`` for the same directory. A work tree
    named one way and an output written the other way would compare unequal and
    the pack would land among tracked files.

    ``os.path.samefile`` answers from the device and inode instead, which is
    what this guard means by the same directory. It needs both paths to exist,
    so the caller keeps plain equality alongside it for the parts of an output
    path the writer has not created yet. Those cannot be the work tree anyway,
    because a work tree Git is using exists.

    Only a path that is genuinely not there answers no. Every other failure,
    a permission error, a stale mount, an over-long path, is an inspection
    this process could not complete, and answering no to it would tell the
    caller these are different directories on no evidence. Those propagate to
    require_output_outside_repository, which refuses. Same rule as
    _marker_present, and for the same reason.
    """
    try:
        return os.path.samefile(candidate, work_tree)
    except (FileNotFoundError, NotADirectoryError):
        return False


def _enclosing_repository(directory: Path) -> tuple[Path, str] | None:
    """Return the checkout root holding directory and why it counts, or None.

    A marker is a directory in an ordinary checkout, and ``.git`` is a file in
    a worktree or a submodule, so presence is the test rather than is_dir. The
    directory itself counts: an --output pointed at a checkout root is inside
    that checkout.

    The marker scan runs first, because when both apply the marker is the more
    useful thing to name in the refusal.

    Raises OSError when a candidate cannot be inspected. The caller refuses on
    it: a directory this process cannot inspect is not one it can show to be
    outside a checkout.
    """
    for candidate in (directory, *directory.parents):
        for marker in CHECKOUT_MARKERS:
            if _marker_present(candidate, marker):
                return candidate, f"which holds {marker}"
    work_tree = _configured_work_tree()
    if work_tree is not None:
        for candidate in (directory, *directory.parents):
            if candidate == work_tree or _same_directory(candidate, work_tree):
                return (
                    work_tree,
                    "which the GIT_WORK_TREE environment variable names",
                )
    return None


def require_output_outside_repository(output_dir: Path) -> Path:
    """Refuse an output directory inside a version-control checkout, and return it.

    A readiness pack names a client's file, its workpaper references and every
    finding standing between it and manager review. Inside a checkout it is one
    ``git add -A`` away from a history that is copied to every clone and, on a
    public remote, to everyone; a .gitignore entry is a convention the next
    commit can waive, and it does nothing about the copy sitting in the working
    tree in the meantime.

    The return value is the resolved directory, and it is what the caller must
    then write to. Checking one path and writing to another leaves the two free
    to disagree: ``resolve`` follows every symlink in the path once, while each
    later ``mkdir`` and ``write_text`` follows them again, so a component
    re-pointed in between would send the pack somewhere this function never
    approved. That narrows the window rather than closing it; a component
    replaced between the directory being created and a file being written still
    redirects that write, and only opening each path component without
    following symlinks would prevent it, which is not available on every
    platform this component supports.

    A path this function cannot examine is refused too, rather than allowed
    through or left to raise. ``resolve`` raises RuntimeError for a
    symbolic-link loop on every Python before 3.13 and returns the unresolved
    path from 3.13, so without the reachability check the command would answer
    a bad --output with a traceback on one interpreter and a later mkdir
    failure on another, where the README promises a message and exit 1. The
    marker scan uses lstat for the same reason from the other direction: it
    must not decide that a marker it cannot read is a marker that is not there.

    The check is a refusal rather than a warning, and there is no override,
    because every other gate in this package fails closed and because the
    caller who most needs it is the one who did not think about it. A pack
    already written into a checkout still opens: ``view`` only reads.
    """
    try:
        resolved = output_dir.resolve()
        _reject_unreachable(resolved)
        checkout = _enclosing_repository(resolved)
    except (OSError, RuntimeError, ValueError) as exc:
        raise GateInputError(
            f"{output_dir} cannot be examined for an enclosing version-control "
            f"checkout: {exc}. An output this run cannot inspect is refused "
            "rather than written to, because a readiness pack names a client's "
            "file and its open findings. Point --output somewhere readable."
        ) from exc
    if checkout is None:
        return resolved
    repository, reason = checkout
    raise GateInputError(
        f"{output_dir} is inside the version-control checkout at {repository}, "
        f"{reason}. A readiness pack names a client's file and its "
        "open findings, so it belongs in an access-controlled directory outside "
        "version control. Point --output somewhere outside that checkout."
    )


def write_review_pack(pack: ReadinessPack, output_dir: Path) -> dict[str, Path]:
    """Write the three pack files, refusing an output inside a checkout first.

    The refusal happens before any directory is created, so a rejected run
    leaves nothing behind, and every destination below is built from the
    directory the guard approved rather than from the argument, so the location
    that was checked is the location written to. The check lives here rather
    than only in the CLI so that a library caller cannot reach the writer
    without passing it.
    """
    output_dir = require_output_outside_repository(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    json_path = output_dir / "readiness-pack.json"
    summary_path = output_dir / "readiness-summary.md"
    findings_path = output_dir / "findings.csv"
    rendered = (
        (json_path, json.dumps(_as_json(pack), indent=2, sort_keys=True) + "\n", "utf-8", None),
        (summary_path, _as_markdown(pack), "utf-8", None),
        (findings_path, _as_csv(pack), "utf-8-sig", ""),
    )

    staged: list[tuple[Path, Path]] = []
    try:
        for destination, text, encoding, newline in rendered:
            staged_path = _sibling_partial(destination)
            staged.append((staged_path, destination))
            staged_path.write_text(text, encoding=encoding, newline=newline)
    except BaseException:
        for staged_path, _ in staged:
            _remove_quietly(staged_path)
        raise

    replaced: list[tuple[Path, Path | None]] = []
    try:
        for staged_path, destination in staged:
            replaced.append((destination, _swap_into_place(staged_path, destination)))
    except OSError:
        for destination, parked in reversed(replaced):
            if parked is None:
                _remove_quietly(destination)
            else:
                _restore_quietly(parked, destination)
        for staged_path, _ in staged:
            _remove_quietly(staged_path)
        raise
    for _, parked in replaced:
        if parked is not None:
            _remove_quietly(parked)
    return {"json": json_path, "summary": summary_path, "findings": findings_path}
