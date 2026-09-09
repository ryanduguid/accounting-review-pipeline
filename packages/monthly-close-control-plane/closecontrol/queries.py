"""Derive the client-query register from the exceptions one run raised.

An exception says what the control found. A query says what somebody has to
ask, and of whom. The two are not the same list: a trial balance that does not
balance is the firm's export to fix, while a material movement nobody can
explain from the ledger is a question only the client's records or people can
settle. Sending the first to a client wastes the client's time and the firm's
credibility; leaving the second inside a variance table means it gets asked
late, by whoever notices.

Nothing here decides anything. Every query is a draft question for the preparer
to read, edit and put to the client through the firm's own channel, and no
query is answered, closed or sent by this package.
"""

from __future__ import annotations

import hashlib

from .errors import ControlInputError
from .models import ClientQuery, ExceptionItem


# Controls whose exceptions never become a client query, and why. A control
# absent from both this mapping and CLIENT_ANSWERABLE_CONTROLS is unclassified,
# which _classify treats as firm-resolved: a new control raises no client
# question until somebody decides it should.
FIRM_RESOLVED_CONTROLS = {
    "trial_balance_integrity": (
        "The supplied trial balance does not balance. The firm corrects or "
        "re-exports it; the client cannot answer for an export they did not make."
    ),
    "financial_year_reset": (
        "The two report dates straddle 30 June. The firm chose the comparison "
        "and reruns it inside one financial year."
    ),
    "account_mapping": (
        "The review-group mapping is the firm's own reporting file. An unmapped "
        "account is the firm's to assign."
    ),
}


# The question and the evidence that would answer it, per client-answerable
# control. The text carries no amounts or account names: those travel in the
# query's own columns, so one template renders correctly for every account and
# the money formatting stays in the report layer with every other figure.
CLIENT_ANSWERABLE_CONTROLS = {
    "period_variance": (
        "",
        "What drove the year-to-date movement in this account against the prior period?",
        "The transactions or documents behind the movement, and confirmation that "
        "no part of it belongs to another period.",
    ),
    "account_metadata": (
        "",
        "Who changed this account's code, name or section since the prior period, and why?",
        "The date of the chart-of-accounts change, who approved it, and any effect "
        "on how the prior period was reported.",
    ),
}

# period_comparison and subledger_reconciliation each raise two different
# situations under one control name, so their question depends on the shape of
# the exception rather than on the control alone. The engine leaves
# current_value empty in exactly one of each pair, which is what _classify
# tests: an absent current balance means the account or the ledger side is
# missing, a present one means both sides exist and disagree.
_ACCOUNT_ABSENT = (
    "absent",
    "Was this account closed, reclassified or renamed during the period, or is it "
    "missing from the export?",
    "Confirmation of the change and the account the balance moved to, or a corrected export.",
)
_ACCOUNT_NEW = (
    "new",
    "What is this new account used for, and when did it start being used?",
    "The reason the account was opened and the first transaction posted to it.",
)
_SUBLEDGER_DIFFERENCE = (
    "difference",
    "What makes up the difference between the general ledger balance and the "
    "subledger balance for this account?",
    "The reconciling items with their dates and amounts, and support for any that "
    "are timing differences.",
)


# Appended to a period_variance query when the pack also carries the
# financial_year_reset exception. Year-to-date figures for profit-and-loss
# accounts restart on 1 July, so a comparison across the reset shows a full
# year against one or two months and the movement is an artefact of the dates,
# not the business. The engine says it cannot tell which rows reset without
# section rules it does not have, so the register does not guess either: it
# says so on the rows that could be affected, because asking a client to
# explain a reset is the fastest way to lose their confidence in the question.
_YEAR_RESET_CAVEAT = (
    " This comparison crosses a 30 June year-to-date reset, so confirm the movement "
    "is real before putting this query to anyone: a profit-and-loss account restarts "
    "at nil on 1 July."
)


def _classify(item: ExceptionItem) -> tuple[str, str, str] | None:
    """Return the variant tag, question and evidence, or None for no query.

    The variant distinguishes two questions raised under one control name. It
    is part of the query's identity, because an account that disappears one
    period and returns later asks the firm two different things, and a tracker
    holding both under one number cannot tell them apart.
    """
    if item.control in FIRM_RESOLVED_CONTROLS:
        return None
    if item.control == "period_comparison":
        return _ACCOUNT_ABSENT if item.current_value is None else _ACCOUNT_NEW
    if item.control == "subledger_reconciliation":
        # A subledger balance with no matching trial-balance account is a
        # mapping or source fault the firm settles before anyone asks the
        # client to explain a difference between two figures that are not yet
        # comparable.
        return _SUBLEDGER_DIFFERENCE if item.current_value is not None else None
    return CLIENT_ANSWERABLE_CONTROLS.get(item.control)


# Twelve hex characters, not eight. The identifier has to survive being carried
# into a firm's tracker, so the space it is drawn from should not be one a
# register could plausibly fill: 48 bits leaves a collision beyond any number
# of accounts a close produces, and the guard in derive_client_queries is then
# a genuine invariant check rather than a failure mode to design around.
_ID_LENGTH = 12


def _query_id(item: ExceptionItem, variant: str) -> str:
    """A short identifier that stays the same while the question stands open.

    Derived from the control, the account and the question's variant rather
    than from the query's position, so a firm that carried Q-3f2a1b0c9d8e into
    last month's list sees the same identifier this month if the same question
    recurs, and an unrelated exception appearing earlier in the pack does not
    renumber it.

    Each part is length-prefixed because a bare separator does not distinguish
    the parts it joins: the loader admits a pipe in a tenant and in an account
    id, so joining on one alone gives tenant "A|B" with account "C" the same
    material as tenant "A" with account "B|C". Two unrelated accounts would
    then draw the same identifier, and the duplicate guard below would abort
    the whole pack while naming a cause that never happened.
    """
    parts = (item.control, variant, item.tenant, item.account_id)
    material = "|".join(f"{len(part)}:{part}" for part in parts)
    return "Q-" + hashlib.sha256(material.encode("utf-8")).hexdigest()[:_ID_LENGTH]


def derive_client_queries(
    exceptions: tuple[ExceptionItem, ...] | list[ExceptionItem],
) -> tuple[ClientQuery, ...]:
    """Build the register, preserving the order the exceptions were raised in.

    Raises ControlInputError if two queries would share an identifier. Among
    the client-answerable controls the engine raises at most one exception per
    account and variant, so a collision means a control now raises two, and a
    register with one number against two questions is worse than no register:
    a client answers one of them and the firm cannot tell which. review_close
    forces this derivation once, so the condition is reported on the same
    failure path as a malformed input rather than escaping the pack writer,
    whose caller handles only OSError and ValueError.
    """
    crosses_year_reset = any(
        item.control == "financial_year_reset" for item in exceptions
    )
    queries: list[ClientQuery] = []
    seen: dict[str, ExceptionItem] = {}
    for item in exceptions:
        classified = _classify(item)
        if classified is None:
            continue
        variant, question, evidence = classified
        if crosses_year_reset and item.control == "period_variance":
            evidence += _YEAR_RESET_CAVEAT
        query_id = _query_id(item, variant)
        previous = seen.get(query_id)
        if previous is not None:
            raise ControlInputError(
                f"two client queries share {query_id}: control {item.control!r} "
                f"raised more than one exception for account {item.account_id!r}"
            )
        seen[query_id] = item
        queries.append(
            ClientQuery(
                query_id=query_id,
                control=item.control,
                tenant=item.tenant,
                account_id=item.account_id,
                account_code=item.account_code,
                account_name=item.account_name,
                review_group=item.review_group,
                difference=item.difference,
                question=question,
                evidence_requested=evidence,
            )
        )
    return tuple(queries)
