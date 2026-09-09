from __future__ import annotations

import csv
import io
import json
import os
import uuid
from pathlib import Path

from .engine import CloseReviewPack
from .errors import ControlInputError
from .models import ExceptionItem


def _money(value) -> str:
    """Render a monetary amount with at least two decimal places, never fewer than it has.

    A fixed two-place render reports a 0.0040 difference against a 0.001
    tolerance as "0.00 exceeds 0.00" and leaves the reviewer no way back to the
    real figures, which defeats the exact-decimal arithmetic behind them.
    A Decimal drives the scale from its own exponent; an int or a float, which
    a library caller may still put in an ExceptionItem, renders at two places.
    """
    if value is None:
        return ""
    as_tuple = getattr(value, "as_tuple", None)
    places = 2
    if as_tuple is not None:
        exponent = as_tuple().exponent
        if isinstance(exponent, int):
            places = max(2, -exponent)
    return f"{value:.{places}f}"


def _percentage(value) -> str:
    """Render a ratio as a percentage at two places, or enough to show its leading digit.

    A percentage threshold finer than a hundredth of a per cent otherwise reads
    as "0.00%" in all three files, the same loss of the configured figure that
    _money exists to avoid. The scale follows the leading significant digit
    rather than the exponent, because percentage_change is a division result
    carrying the full decimal context precision and would render 28 places.
    """
    if value is None:
        return ""
    scaled = value * 100
    adjusted = getattr(scaled, "adjusted", None)
    places = max(2, -adjusted()) if adjusted is not None else 2
    return f"{scaled:.{places}f}%"


def _exception_dict(item: ExceptionItem) -> dict[str, str]:
    return {
        "control": item.control,
        "status": item.status,
        "tenant": item.tenant,
        "account_id": item.account_id,
        "account_code": item.account_code,
        "account_name": item.account_name,
        "review_group": item.review_group,
        "current_value": _money(item.current_value),
        "prior_value": _money(item.prior_value),
        "difference": _money(item.difference),
        "threshold": _money(item.threshold),
        "percentage_change": _percentage_cell(item),
        "reason": item.reason,
        "reviewer_action": item.reviewer_action,
    }


def _csv_safe(value: str) -> str:
    """Keep source-controlled text inert when an exceptions CSV is opened in a spreadsheet.

    Spreadsheet applications can interpret text beginning with '=', '+', '-',
    or '@' as a formula, including after leading whitespace. Prefix every such
    source value with an apostrophe. Numeric report fields do not pass through
    this helper, so legitimate monetary and percentage outputs remain numeric
    text in the CSV. An already guarded value begins with an apostrophe and is
    returned unchanged.
    """
    stripped = value.lstrip()
    if stripped.startswith(("=", "+", "-", "@")):
        return "'" + value
    return value


def _as_json(pack: CloseReviewPack) -> dict:
    acknowledgement = None
    if pack.acknowledgement is not None:
        acknowledgement = {
            "reviewer_initials": pack.acknowledgement.reviewer_initials,
            "reviewed_on": pack.acknowledgement.reviewed_on.isoformat(),
            "comment": pack.acknowledgement.comment,
            "effect": "Acknowledgement is evidence of human review only; it does not approve or close a period.",
        }
    return {
        "acknowledgement": acknowledgement,
        "current_report_dates": list(pack.current_report_dates),
        "exceptions": [_exception_dict(item) for item in pack.exceptions],
        "overall_status": pack.status,
        "prior_report_dates": list(pack.prior_report_dates),
        "source_sha256": dict(sorted(pack.source_hashes.items())),
        "thresholds": {
            "absolute_variance": _money(pack.absolute_threshold),
            "percentage_variance": _percentage(pack.percentage_threshold),
            "reconciliation_tolerance": _money(pack.reconciliation_tolerance),
        },
    }


# A missing tenant, account or difference is shown as text rather than as an em
# dash: runtime output stays ASCII so a pack still reads on a console or in a
# scheduler log whose code page has no dash to render.
_ABSENT = "n/a"

# A period_variance exception whose prior YTD balance was nil has no percentage
# to render: the engine leaves percentage_change as None and its reason names
# the absolute gate as the only one tested. A blank cell in that position reads
# as "no change", so the pack states the condition instead. The sentinel keeps
# the existing ASCII "n/a" convention and cannot parse as a number, so a
# spreadsheet or JSON consumer cannot mistake it for a zero percentage.
_PRIOR_ZERO_PERCENTAGE = "n/a (prior period zero)"


def _percentage_cell(item: ExceptionItem) -> str:
    """Render an exception's percentage_change, naming the prior-zero case.

    Only a period_variance exception with a nil prior value gets the sentinel:
    every other control (integrity, mapping, metadata, reconciliation) carries
    percentage_change=None because a percentage is not part of that control at
    all, and those cells stay empty as before.
    """
    if (
        item.percentage_change is None
        and item.control == "period_variance"
        and item.prior_value is not None
        and item.prior_value == 0
    ):
        return _PRIOR_ZERO_PERCENTAGE
    return _percentage(item.percentage_change)


def _md_cell(value: str) -> str:
    r"""Flatten embedded newlines and escape a value so it cannot break a table row.

    Backslashes are escaped before pipes. A Markdown parser reads a backslash as
    consuming the character after it, so escaping the pipe alone turns source
    text of `\|` into `\\|` - an escaped backslash followed by a live delimiter -
    which adds a cell to the row and shifts every column after it. Doubling the
    backslash first leaves `\\\|`, which reads as one backslash and one literal
    pipe. Order matters: escaping pipes first and backslashes second would
    re-escape the backslashes this function just added.
    """
    return " ".join(value.split()).replace("\\", "\\\\").replace("|", "\\|").replace("*", "\\*").replace("`", "\\`")


def _md_note_lines(comment: str) -> list[str]:
    r"""Render a reviewer comment without collapsing its line structure.

    A one-line comment stays inline on the list item. A multi-line comment is
    rendered as an indented blockquote under the item, one quoted line per
    source line, so a reviewer's paragraph breaks survive into the pack.
    Each line still gets the backslash-then-pipe escaping of _md_cell, and a
    line that starts with '#' is escaped so quoted text cannot forge a
    document heading. The quote marker keeps every continuation line inside
    the acknowledgement item rather than loose in the document.
    """
    if "\n" not in comment and "\r" not in comment:
        return [f"- Comment: {_md_cell(comment)}"]
    lines = ["- Comment:"]
    for raw in comment.splitlines():
        line = _md_cell(raw)
        if line.startswith("#"):
            line = "\\" + line
        lines.append(f"  > {line}".rstrip())
    return lines


def _as_markdown(pack: CloseReviewPack) -> str:
    blocked = sum(item.status == "BLOCKED" for item in pack.exceptions)
    review = sum(item.status == "REVIEW" for item in pack.exceptions)
    lines = [
        "# Monthly Close Review Pack",
        "",
        f"**Overall status: {pack.status}**",
        "",
        "This pack is a review aid. It does not approve a close, post a journal, make a payment, lodge a return, or lock a period.",
        "",
        "## Scope",
        "",
        f"- Current report date(s): {', '.join(pack.current_report_dates)}",
        f"- Prior report date(s): {', '.join(pack.prior_report_dates)}",
        f"- Material variance thresholds: ${_money(pack.absolute_threshold)} and {_percentage(pack.percentage_threshold)}",
        f"- Reconciliation tolerance: ${_money(pack.reconciliation_tolerance)}",
        f"- Exceptions: {len(pack.exceptions)} total; {blocked} blocked; {review} requiring review.",
        "",
        "## Source evidence",
        "",
    ]
    for name, digest in sorted(pack.source_hashes.items()):
        lines.append(f"- `{name}`: `{digest}`")
    lines += ["", "## Exceptions", ""]
    if not pack.exceptions:
        lines.append("No exceptions were raised. A human must still decide whether the close is appropriate.")
    else:
        lines += [
            "| Status | Control | Tenant | Account | Difference | Reason |",
            "| --- | --- | --- | --- | ---: | --- |",
        ]
        for item in pack.exceptions:
            account = " / ".join(piece for piece in (item.account_code, item.account_name) if piece) or item.account_id or _ABSENT
            account = _md_cell(account)
            reason = _md_cell(item.reason)
            tenant = _md_cell(item.tenant or _ABSENT)
            lines.append(f"| {item.status} | {item.control} | {tenant} | {account} | {_money(item.difference) or _ABSENT} | {reason} |")
    lines += ["", "## Human acknowledgement", ""]
    if pack.acknowledgement is None:
        lines.append("No reviewer acknowledgement was supplied. This does not create or imply an approval.")
    else:
        lines += [
            f"- Reviewer initials: {_md_cell(pack.acknowledgement.reviewer_initials)}",
            f"- Reviewed on: {pack.acknowledgement.reviewed_on.isoformat()}",
            *_md_note_lines(pack.acknowledgement.comment),
            "- Effect: acknowledgement records a human action only; it does not change the control status or approve a close.",
        ]
    lines.append("")
    return "\n".join(lines)


def _as_csv(pack: CloseReviewPack) -> str:
    fields = list(_exception_dict(ExceptionItem("", "PASS", "", "", "", "", None, None, None, None, None, "", "")).keys())
    buffer = io.StringIO()
    writer = csv.DictWriter(buffer, fieldnames=fields)
    writer.writeheader()
    for item in pack.exceptions:
        row = _exception_dict(item)
        for field in ("tenant", "account_id", "account_code", "account_name", "review_group"):
            row[field] = _csv_safe(row[field])
        writer.writerow(row)
    return buffer.getvalue()


def _remove_quietly(path: Path) -> None:
    try:
        path.unlink()
    except OSError:
        # Cleanup is best effort; the caller re-raises the original failure.
        pass


def _restore_quietly(parked: Path, destination: Path) -> None:
    """Put a previous run's file back where it was. Best effort, as above."""
    try:
        os.replace(parked, destination)
    except OSError:
        pass


def _sibling_partial(destination: Path) -> Path:
    """Create an empty, uniquely named file beside `destination` to hold pack content.

    A fixed `<name>.partial` lets two runs sharing an --output directory
    overwrite each other's staged file and then move the wrong pack into place.
    The file is created by an ordinary exclusive open rather than by
    tempfile.mkstemp, because a staged file becomes the pack file and mkstemp's
    owner-only mode would quietly change who can read a delivered pack.
    """
    while True:
        candidate = destination.with_name(f"{destination.name}.{uuid.uuid4().hex[:12]}.partial")
        try:
            with candidate.open("x", encoding="utf-8"):
                pass
        except FileExistsError:  # pragma: no cover - a 48-bit name collision.
            continue
        return candidate


def _swap_into_place(staged_path: Path, destination: Path) -> Path | None:
    """Move a staged file onto its destination, parking any previous content aside.

    Returns the file now holding the previous content so the caller can restore
    it if a later move fails, or None if the destination held no file. Nothing
    is destroyed here: an existing file is renamed, never unlinked.

    Only a regular file is parked. Anything else at the destination - a
    directory, most plainly - stays put and fails the move, because Windows
    renames a directory onto a file path quite happily and moving one aside is
    well beyond writing a review pack.
    """
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


PACK_FILE_NAMES = ("close-review-pack.json", "close-summary.md", "exceptions.csv")
"""The three names write_review_pack claims in its output directory."""


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

    The return value is the resolved directory, and it is what the caller must
    then write to. Checking one path and writing to another leaves the two free
    to disagree: ``resolve`` follows every symlink in the path once, while each
    later ``mkdir`` and ``write_text`` follows them again, so a component
    re-pointed in between would send the pack somewhere this function never
    approved. Returning the approved path is what binds the decision to the
    destination.

    That narrows the window rather than closing it. A component replaced
    between the directory being created and a file being written still
    redirects that write, and only opening each path component without
    following symlinks would prevent it, which is not available on every
    platform this component supports. The guard is aimed at the careless
    output path, not at a local actor racing the process for it.

    A review pack names a client's accounts, balances and unexplained
    movements. Inside a checkout it is one ``git add -A`` away from a history
    that is copied to every clone and, on a public remote, to everyone; a
    .gitignore entry is a convention the next commit can waive, and it does
    nothing about the copy sitting in the working tree in the meantime.

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
        raise ControlInputError(
            f"{output_dir} cannot be examined for an enclosing version-control "
            f"checkout: {exc}. An output this run cannot inspect is refused "
            "rather than written to, because a review pack names a client's "
            "accounts and balances. Point --output somewhere readable."
        ) from exc
    if checkout is None:
        return resolved
    repository, reason = checkout
    raise ControlInputError(
        f"{output_dir} is inside the version-control checkout at {repository}, "
        f"{reason}. A review pack names a client's accounts and "
        "balances, so it belongs in an access-controlled directory outside "
        "version control. Point --output somewhere outside that checkout."
    )


def write_review_pack(pack: CloseReviewPack, output_dir: Path) -> dict[str, Path]:
    """Write the three pack files so a failed run cannot leave two runs mixed together.

    Each file is rendered in full, staged beside its destination under a unique
    name, and only then moved into place. If a move fails - a locked
    exceptions.csv is the usual cause - the files this run had already moved are
    rolled back to the content they replaced, so the directory holds the whole
    previous pack rather than one file from this run beside two from the last
    one; all three carry the same SHA-256 provenance framing and a reviewer
    cannot tell them apart. Apart from the three destinations themselves, no
    file is ever deleted; a caller that points a source path at one of
    PACK_FILE_NAMES inside output_dir destroys that source, which is why the
    CLI refuses that combination before the run starts.

    Rollback is best effort against a second failure, and a run killed outright
    can leave a `.partial` file behind, so a stray `.partial` may hold either
    this run's staged content or the previous run's. Concurrent runs sharing one
    output directory are not serialised; run one at a time per directory.

    An output directory inside a version-control checkout is refused before any
    directory is created, so a rejected run leaves nothing behind. The check
    lives here rather than only in the CLI so that a library caller cannot
    reach the writer without passing it, and the returned paths are rooted at
    the resolved directory it approved rather than at the argument, so the
    location that was checked is the location written to.

    exceptions.csv carries a UTF-8 byte-order mark to match the canonical input
    files, so a spreadsheet that falls back to the Windows ANSI code page does
    not turn a tenant or account name into mojibake.
    """
    # Every destination below is built from the directory the guard approved,
    # not from the argument, so the path that was checked is the path written.
    output_dir = require_output_outside_repository(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    json_path = output_dir / "close-review-pack.json"
    summary_path = output_dir / "close-summary.md"
    exceptions_path = output_dir / "exceptions.csv"
    rendered = (
        (json_path, json.dumps(_as_json(pack), indent=2, sort_keys=True) + "\n", "utf-8", None),
        (summary_path, _as_markdown(pack), "utf-8", None),
        (exceptions_path, _as_csv(pack), "utf-8-sig", ""),
    )

    staged: list[tuple[Path, Path]] = []
    try:
        for destination, text, encoding, newline in rendered:
            staged_path = _sibling_partial(destination)
            # Recorded before the write, so a write that dies part-way through
            # is cleaned up rather than left as a truncated orphan.
            staged.append((staged_path, destination))
            staged_path.write_text(text, encoding=encoding, newline=newline)
    except BaseException:
        # Not just OSError: a render that cannot be encoded raises
        # UnicodeEncodeError, a ValueError, and used to walk out of here
        # leaving a .partial holding the whole pack - tenant, accounts and
        # balances - in a directory the caller believes the run never wrote to.
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
                # This run created the file; nothing preceded it.
                _remove_quietly(destination)
            else:
                _restore_quietly(parked, destination)
        for staged_path, _ in staged:
            _remove_quietly(staged_path)
        raise
    for _, parked in replaced:
        if parked is not None:
            _remove_quietly(parked)
    return {"json": json_path, "summary": summary_path, "exceptions": exceptions_path}
