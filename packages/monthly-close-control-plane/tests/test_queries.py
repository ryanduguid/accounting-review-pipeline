"""The client-query register: what it asks, what it refuses to ask, and why."""

from __future__ import annotations

from decimal import Decimal
from pathlib import Path

import pytest

from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError
from closecontrol.models import ExceptionItem
from closecontrol.queries import (
    CLIENT_ANSWERABLE_CONTROLS,
    FIRM_RESOLVED_CONTROLS,
    derive_client_queries,
)


EXAMPLES = Path(__file__).resolve().parents[1] / "examples"


def _exception(
    control: str,
    *,
    account_id: str = "1000",
    tenant: str = "Demo Pty Ltd",
    current_value: Decimal | None = Decimal("100.00"),
    prior_value: Decimal | None = Decimal("10.00"),
    difference: Decimal | None = Decimal("90.00"),
) -> ExceptionItem:
    return ExceptionItem(
        control=control,
        status="REVIEW",
        tenant=tenant,
        account_id=account_id,
        account_code="1000",
        account_name="Operating Bank",
        current_value=current_value,
        prior_value=prior_value,
        difference=difference,
        threshold=Decimal("1000.00"),
        percentage_change=None,
        reason="fabricated",
        reviewer_action="fabricated",
    )


def test_firm_resolved_controls_raise_no_client_query() -> None:
    """A client cannot answer for the firm's export, its choice of comparison
    dates, or its own reporting map. Sending those as questions spends the
    client's patience on work the firm has to do itself."""
    exceptions = tuple(_exception(control) for control in FIRM_RESOLVED_CONTROLS)

    assert derive_client_queries(exceptions) == ()


def test_an_unclassified_control_raises_no_query() -> None:
    """A control added later asks nobody anything until somebody decides it
    should. Defaulting the other way would put an unreviewed question in front
    of a client the first time a new control fired."""
    assert derive_client_queries((_exception("some_future_control"),)) == ()


def test_each_client_answerable_control_asks_a_question_and_names_evidence() -> None:
    for control in CLIENT_ANSWERABLE_CONTROLS:
        (query,) = derive_client_queries((_exception(control),))
        assert query.control == control
        assert query.question.endswith("?")
        assert query.evidence_requested
        assert query.difference == Decimal("90.00")


def test_subledger_query_waits_until_both_sides_exist() -> None:
    """A subledger balance with no trial-balance account is a mapping or source
    fault. Asking a client to explain the difference between two figures that
    are not yet comparable produces an answer nobody can use."""
    unmatched = _exception("subledger_reconciliation", current_value=None, difference=None)
    assert derive_client_queries((unmatched,)) == ()

    matched = _exception("subledger_reconciliation")
    (query,) = derive_client_queries((matched,))
    assert "difference between the general ledger balance" in query.question


def test_period_comparison_asks_a_different_question_each_way() -> None:
    gone = _exception("period_comparison", current_value=None, difference=None)
    (closed,) = derive_client_queries((gone,))
    assert "closed, reclassified or renamed" in closed.question

    arrived = _exception("period_comparison", prior_value=None, difference=None)
    (opened,) = derive_client_queries((arrived,))
    assert "What is this new account used for" in opened.question


def test_a_year_reset_caveats_the_variance_queries_it_could_explain() -> None:
    """YTD figures for profit-and-loss accounts restart on 1 July, so a
    comparison across the reset can show a movement that is an artefact of the
    dates. The engine will not guess which rows reset, so neither does the
    register: it says so on the rows that could be affected rather than asking
    a client to explain arithmetic the firm chose."""
    variance = _exception("period_variance")
    (without,) = derive_client_queries((variance,))
    assert "30 June" not in without.evidence_requested

    (with_reset, ) = derive_client_queries((variance, _exception("financial_year_reset")))
    assert "crosses a 30 June year-to-date reset" in with_reset.evidence_requested
    # Only the variance rows carry it: a subledger difference does not reset.
    subledger = _exception("subledger_reconciliation", account_id="2000")
    queries = derive_client_queries((subledger, _exception("financial_year_reset")))
    assert "30 June" not in queries[0].evidence_requested


def test_query_ids_are_stable_across_runs_and_unique_within_one() -> None:
    """A firm carries an identifier into its own tracker and expects the same
    question to keep it next month, so the identifier follows the control and
    the account rather than the row's position in the pack."""
    variance = _exception("period_variance")
    subledger = _exception("subledger_reconciliation", account_id="2000")

    first = derive_client_queries((variance, subledger))
    # The same two questions, reached after an unrelated exception appeared
    # earlier in the pack, keep their identifiers.
    second = derive_client_queries((_exception("account_mapping"), variance, subledger))

    assert [query.query_id for query in first] == [query.query_id for query in second]
    assert len({query.query_id for query in first}) == 2


def test_a_pipe_in_a_tenant_or_account_cannot_collide_two_queries() -> None:
    """The loader admits a pipe in both fields, so joining the identifier's
    parts on one alone gave tenant "A|B" with account "C" the same material as
    tenant "A" with account "B|C". Two unrelated accounts drew one identifier,
    and the duplicate guard then aborted the whole pack naming a cause that
    never happened. Length-prefixing each part settles it whatever the text."""
    left = _exception("subledger_reconciliation", tenant="A|B", account_id="C")
    right = _exception("subledger_reconciliation", tenant="A", account_id="B|C")

    queries = derive_client_queries((left, right))

    assert len({query.query_id for query in queries}) == 2


def test_two_queries_for_one_account_and_control_fail_loudly() -> None:
    """One number against two questions is worse than no register: the client
    answers one of them and the firm cannot tell which.

    ControlInputError, not RuntimeError: review_close forces the derivation, so
    the condition reaches the CLI on the path that already reports an input
    problem instead of escaping the pack writer as a traceback.
    """
    duplicate = _exception("period_variance")
    with pytest.raises(ControlInputError, match="share Q-"):
        derive_client_queries((duplicate, duplicate))


def test_the_two_period_comparison_questions_get_different_identifiers() -> None:
    """An account that disappears one period and returns later asks the firm
    two different things. One identifier against both would let a tracker
    attach a closure answer to a new-account question."""
    gone = _exception("period_comparison", current_value=None, difference=None)
    arrived = _exception("period_comparison", prior_value=None, difference=None)

    (closed,) = derive_client_queries((gone,))
    (opened,) = derive_client_queries((arrived,))
    assert closed.query_id != opened.query_id


def test_the_fabricated_demo_asks_only_what_the_client_can_answer() -> None:
    pack = review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
        mapping_path=EXAMPLES / "account_mapping.csv",
        subledger_path=EXAMPLES / "subledger_balances.csv",
    )

    raised = {item.control for item in pack.exceptions}
    asked = {query.control for query in pack.client_queries}
    assert "account_mapping" in raised and "account_mapping" not in asked
    assert "financial_year_reset" in raised and "financial_year_reset" not in asked
    assert asked == {"period_variance", "subledger_reconciliation"}
    assert len({query.query_id for query in pack.client_queries}) == len(pack.client_queries)


def test_the_register_is_derived_so_it_cannot_contradict_the_exceptions() -> None:
    """client_queries is a property, not a stored field, so a pack assembled by
    a library caller cannot carry a register describing different exceptions."""
    pack = review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
    )

    assert pack.client_queries == derive_client_queries(pack.exceptions)
