"""Check-digit vectors are documented ATO/ASIC test identifiers, not real ones.

They live in test code only. Nothing under evatt/samples/ carries them.
"""
import re
import time

from evatt import patterns

VALID_TFN = "123456782"
VALID_ABN = "51824753556"
VALID_ACN = "000000019"
VALID_MEDICARE = "2123456701"


def test_valid_check_digits_are_accepted() -> None:
    assert patterns.valid_tfn(VALID_TFN)
    assert patterns.valid_abn(VALID_ABN)
    assert patterns.valid_acn(VALID_ACN)
    assert patterns.valid_medicare(VALID_MEDICARE)


def test_one_digit_changed_is_rejected() -> None:
    assert not patterns.valid_tfn("123456783")
    assert not patterns.valid_abn("51824753557")
    assert not patterns.valid_acn("000000018")
    # Index 8 is Medicare's check digit. Index 9 is the issue number and takes
    # no part in the sum, so changing the last digit must NOT invalidate it.
    assert not patterns.valid_medicare("2123456711")


def test_the_medicare_issue_number_is_not_part_of_the_check() -> None:
    assert patterns.valid_medicare("2123456701")
    assert patterns.valid_medicare("2123456702")


def test_wrong_length_is_rejected() -> None:
    assert not patterns.valid_tfn("12345678")
    assert not patterns.valid_abn("5182475355")
    assert not patterns.valid_acn("00000001")
    assert not patterns.valid_medicare("212345670")


def test_medicare_rejects_a_leading_digit_outside_two_to_six() -> None:
    assert not patterns.valid_medicare("7123456701")


def test_structured_spans_find_a_labelled_tfn() -> None:
    spans = patterns.structured_spans(f"TFN: {VALID_TFN}")
    assert [(kind, text) for _s, _e, kind, text in spans] == [("tfn", VALID_TFN)]


def test_structured_spans_find_a_spaced_abn() -> None:
    spans = patterns.structured_spans("ABN 51 824 753 556 applies")
    assert [kind for _s, _e, kind, _t in spans] == ["abn"]


def test_structured_spans_find_email_and_phone() -> None:
    text = "Reach a.person@example.com or 0412 345 678 today"
    kinds = sorted(kind for _s, _e, kind, _t in patterns.structured_spans(text))
    assert kinds == ["email", "phone"]


def test_a_nine_digit_run_valid_as_both_is_reported_as_a_tfn() -> None:
    """Safe direction: on an exact tie the more sensitive kind wins.

    000000019 satisfies both the TFN mod-11 sum and the ASIC ACN check digit,
    so it is the vector that actually exercises the tie-break.
    """
    assert patterns.valid_tfn(VALID_ACN)
    assert patterns.valid_acn(VALID_ACN)
    spans = patterns.structured_spans("000 000 019")
    assert [kind for _s, _e, kind, _t in spans] == ["tfn"]


def test_the_sample_acn_is_not_also_a_valid_tfn() -> None:
    """Pins the fixture choice in evatt/samples/identifiers.md.

    If this ever fails, that sample can no longer produce an "acn" count and
    Task 7's kind-coverage test will fail with it.
    """
    assert patterns.valid_acn("123456780")
    assert not patterns.valid_tfn("123456780")


def test_negatives_from_the_source_module_do_not_trigger() -> None:
    """Cases the origin module already paid a review cycle to learn."""
    for text in (
        "see section 12345678 of the Act",
        "the balance was 1,234,567 at year end",
        "in 2024 the rate changed",
        "Division 7A applies",
    ):
        assert patterns.structured_spans(text) == [], text


def test_bsb_requires_the_hyphen() -> None:
    assert [k for _s, _e, k, _t in patterns.structured_spans("BSB 062-000")] == ["bsb"]
    assert patterns.structured_spans("total 062000 units") == []


def test_person_names_survive_and_statutory_vocabulary_does_not() -> None:
    assert "Jane Roe" in patterns.person_names("Jane Roe attended")
    assert patterns.person_names("The Commissioner of Taxation decided") == set()


def test_person_names_handle_all_caps_and_curly_apostrophes() -> None:
    assert "SMITH, John" in patterns.person_names("SMITH, John was present")
    assert "Mary O\u2019Brien" in patterns.person_names("Mary O\u2019Brien signed")


def test_address_and_date_of_birth_shapes() -> None:
    assert patterns.ADDRESS.search("12 Hunter Street was sold")
    assert patterns.DOB.search("born 14 March 1982 in Newcastle")


def test_placeholder_pattern_matches_assigned_placeholders() -> None:
    assert patterns.PLACEHOLDER.fullmatch("CLIENT_01")
    assert patterns.PLACEHOLDER.fullmatch("TFN_12")
    assert not patterns.PLACEHOLDER.fullmatch("CLIENT")


def test_a_labelled_tfn_is_reported_although_its_check_digit_fails() -> None:
    """The label is the evidence; only a bare run has to earn its place.

    A transcription error in a real TFN is still a real TFN, so a labelled one
    is reported either way. The same digits with no label are an ordinary
    number until the check digit says otherwise.
    """
    spans = patterns.structured_spans("TFN: 123456783")
    assert [(kind, text) for _s, _e, kind, text in spans] == [("tfn", "123456783")]
    assert patterns.structured_spans("123456783") == []


def test_labelled_abn_acn_and_medicare_are_reported_without_a_check_digit() -> None:
    for text, kind, value in (
        ("ABN: 51824753557", "abn", "51824753557"),
        ("ABN 51 824 753 557", "abn", "51 824 753 557"),
        ("A.B.N. 51 824 753 557", "abn", "51 824 753 557"),
        ("ACN: 123456781", "acn", "123456781"),
        ("A.C.N. 123 456 781", "acn", "123 456 781"),
        ("Medicare number 2123456711", "medicare", "2123456711"),
        ("Medicare no. 2123456711", "medicare", "2123456711"),
        ("Medicare 2123456711", "medicare", "2123456711"),
    ):
        spans = patterns.structured_spans(text)
        assert [(k, t) for _s, _e, k, t in spans] == [(kind, value)], text
        # Every vector above fails its check digit, so the same digits with the
        # label taken away must report nothing.
        assert patterns.structured_spans(value) == [], value


def test_a_label_wins_the_tie_against_a_bare_run_over_the_same_digits() -> None:
    """000000019 satisfies both the TFN and the ACN check.

    Labelled and bare capture the identical span, so only the order in
    ``_STRUCTURED`` decides it. The label names the kind and must win.
    """
    assert patterns.valid_tfn("000000019")
    assert patterns.valid_acn("000000019")
    for text, kind in (("ACN: 000000019", "acn"), ("TFN: 000000019", "tfn")):
        assert [k for _s, _e, k, _t in patterns.structured_spans(text)] == [kind], text


def test_month_names_no_longer_suppress_a_person() -> None:
    """The twelve months are out of the statutory list; they hid real people."""
    for text, name in (
        ("June Smith attended", "June Smith"),
        ("April Jones signed", "April Jones"),
        ("August Meyer called", "August Meyer"),
        ("Ray May paid the invoice", "Ray May"),
    ):
        assert name in patterns.person_names(text), text
    assert not patterns.is_statutory("June Smith")


def test_the_remaining_statutory_vocabulary_still_filters() -> None:
    for text in (
        "Federal Court of Australia",
        "Part IVA applies",
        "Income Tax Assessment Act",
        "Administrative Appeals Tribunal",
        "THE COMMISSIONER OF TAXATION",
    ):
        # Assert NAME fires first, or the filter assertion below proves nothing.
        assert patterns.NAME.search(text), text
        assert patterns.person_names(text) == set(), text


def test_placeholder_requires_a_left_boundary() -> None:
    """Task 5 reaches for PLACEHOLDER with ``search``, not ``fullmatch``."""
    assert not patterns.PLACEHOLDER.search("XCLIENT_01")
    assert not patterns.PLACEHOLDER.search("MY_CLIENT_01")
    assert patterns.PLACEHOLDER.search("see CLIENT_01 in the workpaper")


def test_address_and_date_of_birth_cover_the_added_shapes() -> None:
    assert patterns.ADDRESS.search("42 Wattle Grove was sold")
    assert patterns.DOB.search("date of birth 14/03/1982")
    assert patterns.DOB.search("date of birth 14-03-1982")
    # Task 5's fixtures.
    assert patterns.ADDRESS.search("12 Hunter Street was sold")
    assert patterns.DOB.search("Born 14 March 1982")
    # yyyy-mm-dd stays out: it collides with accounting period labels.
    assert not patterns.DOB.search("period ending 2024-03-14")


def test_a_grouped_amount_tail_is_accepted_over_detection() -> None:
    """Deliberate over-detection, not an oversight: this ACN is a false positive.

    123 456 780 satisfies the ASIC check digit, so the tail of "$1 123 456 780"
    reports as an ACN. Only the two-character money guard would suppress it,
    and that guard also deletes every detection pinned in the table-row test
    below. Under-detection is the direction that matters, so no bare pattern
    carries that guard and this false positive is the accepted price: one
    placeholder in a private file against a leak.
    """
    # The premises, asserted here so the vector cannot rot: the tail has to
    # satisfy the ACN check and fail the TFN one, or the more sensitive kind
    # takes the span and this pins nothing about ACN.
    assert patterns.valid_acn("123456780")
    assert not patterns.valid_tfn("123456780")
    spans = patterns.structured_spans("paid $1 123 456 780 today")
    assert [(kind, text) for _s, _e, kind, text in spans] == [("acn", "123 456 780")]
    # The one-character guard still does its own job. 123456780 is a valid ACN,
    # so this vector reports one the moment ``(?<![\d$])`` is dropped, unlike
    # the "invoice $1 234 567 890 paid" it replaces: that one failed every
    # check digit and so passed with or without any guard at all.
    assert patterns.valid_acn("123456780")
    assert patterns.structured_spans("total $123456780 owing") == []


def test_a_grouped_amount_tail_valid_as_a_tfn_is_accepted_over_detection() -> None:
    """The same accepted price, now paid by TFN_BARE as well.

    The tail of "$1 123 456 782" satisfies the ATO mod-11 sum, so it is
    redacted as a TFN. Roughly one nine-digit tail in eleven will. Dropping the
    origin's second lookbehind is what admits it, and the trade is deliberate:
    over-redaction costs one placeholder in a private file, under-detection
    leaks a tax file number.
    """
    # The premises, asserted here so the vector cannot rot: the tail has to
    # satisfy the TFN check and fail the ACN one, or this would pass on the
    # wrong kind and prove nothing about TFN_BARE.
    assert patterns.valid_tfn(VALID_TFN)
    assert not patterns.valid_acn(VALID_TFN)
    spans = patterns.structured_spans("paid $1 123 456 782 today")
    assert [(kind, text) for _s, _e, kind, text in spans] == [("tfn", "123 456 782")]
    # The one-character guard still does its own job: no run may start on a
    # digit or a "$", so the same valid TFN behind a "$" reports nothing.
    assert patterns.structured_spans("total $123456782 owing") == []


def test_table_row_and_dated_prose_identifiers_are_detected() -> None:
    """The two shapes a workpaper is full of, pinned against the guard returning.

    A two-character money guard on ABN, ACN and MEDICARE reports nothing for
    any of these: the preceding "7 ", "2 " or "9 " is indistinguishable from
    the interior of a grouped amount.
    """
    for text, kind, value in (
        ("in 2019 2123456701 was issued", "medicare", "2123456701"),
        ("row 7 123456780", "acn", "123456780"),
        ("Entity 2 51824753556", "abn", "51824753556"),
        ("Entity 2 51 824 753 556", "abn", "51 824 753 556"),
    ):
        spans = patterns.structured_spans(text)
        assert [(k, t) for _s, _e, k, t in spans] == [(kind, value)], text


def test_a_bare_tfn_is_detected_in_the_table_row_and_dated_prose_shapes() -> None:
    """The same two shapes, pinned for the most sensitive identifier of the four.

    Both reported nothing while TFN_BARE kept the origin's second lookbehind:
    the preceding "7 " and "9 " are indistinguishable from the interior of a
    grouped amount. 123456782 satisfies the mod-11 sum, so the check digit is
    not what decides these vectors; only the money guard is.
    """
    assert patterns.valid_tfn(VALID_TFN)
    for text in ("row 7 123456782", "in 2019 123456782 was issued"):
        spans = patterns.structured_spans(text)
        assert [(k, t) for _s, _e, k, t in spans] == [("tfn", VALID_TFN)], text


def test_the_spaced_out_label_does_not_need_its_trailing_dot() -> None:
    """A workpaper writes "A.B.N 51 824 753 556" as readily as "A.B.N.".

    Every vector fails its check digit, so nothing but the label admits it.
    """
    for text, kind, value in (
        ("A.B.N 51824753557", "abn", "51824753557"),
        ("A.B.N 51 824 753 557", "abn", "51 824 753 557"),
        ("A.C.N 123456781", "acn", "123456781"),
        ("A.C.N 123 456 781", "acn", "123 456 781"),
    ):
        spans = patterns.structured_spans(text)
        assert [(k, t) for _s, _e, k, t in spans] == [(kind, value)], text
        assert patterns.structured_spans(value) == [], value


def test_medicare_accepts_a_card_qualifier() -> None:
    """Without "card", "Medicare card 2123456711" falls through to MEDICARE.

    There the check digit rejects it and a real Medicare number is lost, which
    is exactly what the labelled patterns exist to prevent.
    """
    for text, value in (
        ("Medicare card 2123456701", "2123456701"),
        ("Medicare card 2123456711", "2123456711"),
    ):
        spans = patterns.structured_spans(text)
        assert [(k, t) for _s, _e, k, t in spans] == [("medicare", value)], text
    assert not patterns.valid_medicare("2123456711")
    assert patterns.structured_spans("2123456711") == []


def test_every_labelled_pattern_takes_up_to_two_qualifier_words() -> None:
    """One qualifier vocabulary across all four labels, not one per pattern.

    The shape of the label says nothing about which qualifier a typist reaches
    for, so "no." must work after ABN exactly as it does after Medicare. Every
    vector below fails its check digit, so the label and its qualifier are the
    only thing admitting it: if the qualifier stops matching, the digits fall
    through to the bare pattern and a real identifier is lost.
    """
    for text, kind, value in (
        ("Medicare card number 2123456711", "medicare", "2123456711"),
        ("Medicare card no. 2123456711", "medicare", "2123456711"),
        ("Medicare cardholder 2123456711", "medicare", "2123456711"),
        ("ABN no. 51824753557", "abn", "51824753557"),
        ("ACN number 123456781", "acn", "123456781"),
        ("TFN no. 123456783", "tfn", "123456783"),
    ):
        spans = patterns.structured_spans(text)
        assert [(k, t) for _s, _e, k, t in spans] == [(kind, value)], text
        assert patterns.structured_spans(value) == [], value


def test_the_label_separator_accepts_a_hash() -> None:
    """A transcribed card writes "Medicare card # 2123456711"."""
    for text, kind, value in (
        ("Medicare card # 2123456711", "medicare", "2123456711"),
        ("TFN # 123456783", "tfn", "123456783"),
        ("ABN #51824753557", "abn", "51824753557"),
        ("A.C.N. # 123 456 781", "acn", "123 456 781"),
    ):
        spans = patterns.structured_spans(text)
        assert [(k, t) for _s, _e, k, t in spans] == [(kind, value)], text


def test_structured_spans_are_sorted_non_overlapping_and_slice_back() -> None:
    """The three guarantees Task 4's replacement pass depends on."""
    document = (
        "Invoice for Jane Roe, ABN 51 824 753 556, TFN: 123456782, "
        "ACN: 123456780, BSB 062-000, Medicare 2123456701, "
        "a.person@example.com, 0412 345 678."
    )
    spans = patterns.structured_spans(document)
    assert sorted(kind for _s, _e, kind, _t in spans) == [
        "abn",
        "acn",
        "bsb",
        "email",
        "medicare",
        "phone",
        "tfn",
    ]
    assert [start for start, _e, _k, _t in spans] == sorted(
        start for start, _e, _k, _t in spans
    )
    previous_end = 0
    for start, end, _kind, text in spans:
        assert start >= previous_end, spans
        assert document[start:end] == text, spans
        previous_end = end


def _failing_scan_seconds(pattern: re.Pattern[str], label: str, spaces: int) -> float:
    """Best of three searches that must fail after a long run of spaces.

    The trap only shows itself on a failing search. When digits do follow the
    spaces the first greedy path succeeds and nothing backtracks.
    """
    text = label + " " * spaces
    best = float("inf")
    for _ in range(3):
        started = time.perf_counter()
        pattern.search(text)
        best = min(best, time.perf_counter() - started)
    return best


def _best_span_seconds(unit: str, repetitions: int) -> float:
    """Best of three name sweeps over *unit* repeated, for the growth check."""
    text = unit * repetitions
    best = float("inf")
    for _ in range(3):
        started = time.perf_counter()
        patterns.person_name_spans(text)
        best = min(best, time.perf_counter() - started)
    return best


def test_the_labelled_separator_does_not_backtrack_quadratically() -> None:
    """Fails if ``\\s*[.:-]?\\s*`` ever replaces ``\\s*(?:[.:-]\\s*)?``.

    Growth rather than wall clock: quadratic cost quadruples per doubling, so
    four times the input predicts about sixteen times the cost against about
    four for the linear form. Measured here, the quadratic form costs 0.017 s
    at 2,000 spaces, 0.068 s at 4,000 and 0.276 s at 8,000, which puts it near
    1.7 s at the small size below and half a minute at the large one. The
    shipped form takes under 4 ms at the large size.
    """
    for label, pattern in (
        ("TFN", patterns.TFN_LABELLED),
        ("ABN", patterns.ABN_LABELLED),
        ("ACN", patterns.ACN_LABELLED),
        ("Medicare", patterns.MEDICARE_LABELLED),
    ):
        small = _failing_scan_seconds(pattern, label, 20_000)
        # Cheap guard, so a reintroduced trap fails in seconds instead of
        # making the suite sit through the large size.
        assert small < 0.1, (label, small)
        large = _failing_scan_seconds(pattern, label, 80_000)
        # Sixteen would be quadratic. The 10 ms term absorbs scheduler noise at
        # these magnitudes so a loaded machine does not flake it.
        assert large < 8 * small + 0.010, (label, small, large)


def test_a_statutory_word_no_longer_swallows_the_person_beside_it() -> None:
    """NAME takes a third token whenever one is there, and used to lose the pair.

    Every vector below returned nothing at all before the three-token window
    retry. "<Given> <Family> Superannuation" is how an SMSF is named and "the
    Board <Given> <Family>" is ordinary workpaper prose, so both shapes are the
    normal case rather than a curiosity.
    """
    for text, name in (
        ("reviewed by Priya Sharma Superannuation", "Priya Sharma"),
        ("John Smith Superannuation was reviewed", "John Smith"),
        ("Jane Doe Tax return lodged", "Jane Doe"),
        ("Mary Jones Court appearance", "Mary Jones"),
        ("signed by the Board Daniel Okafor", "Daniel Okafor"),
        ("the Commissioner Anna Petrov letter", "Anna Petrov"),
    ):
        assert patterns.person_names(text) == {name}, text


def test_a_statutory_heading_above_a_person_no_longer_orphans_the_family_name() -> None:
    """The ordinary shape of these documents: a heading, then a person below it.

    NAME's ``\\s+`` spans the newline, so "Payroll Tax\\nJohn" is one three-token
    candidate whose windows are both statutory. While the scan resumed at
    ``match.end()`` the candidate took "John" with it, "Smith" was left as a
    single token that no pattern reports, and the whole document came back
    unchanged with an empty manifest and no halt. Resuming at the rejected
    candidate's second token offers "Tax\\nJohn Smith" instead, whose second
    window is the person.
    """
    for text, name in (
        ("## Income Tax\nJohn Smith prepared the return", "John Smith"),
        ("Payroll Tax\nJohn Smith called", "John Smith"),
        ("Deferred Tax\nMary Jones reviewed", "Mary Jones"),
        ("Federal Court\nAnna Petrov appeared", "Anna Petrov"),
        ("Notes to Income Tax\nJohn Smith signed", "John Smith"),
        ("Land Tax\nDaniel Okafor paid", "Daniel Okafor"),
        ("Trust Income\nJane Doe signed", "Jane Doe"),
    ):
        assert name in patterns.person_names(text), text


def test_the_single_line_form_of_that_shape_is_recovered_too() -> None:
    """The same three-token rejection with a space where the newline was.

    This one leaked before the three-token window retry existed and after it,
    because both versions resumed past everything the rejected candidate had
    consumed. Only the resume position fixes it.
    """
    for text, name in (
        ("Income Tax John Smith called on one line", "John Smith"),
        ("Payroll Tax Mary Jones reconciled it", "Mary Jones"),
        ("the Federal Court Anna Petrov appeared", "Anna Petrov"),
    ):
        assert name in patterns.person_names(text), text


def test_the_rejected_candidate_retry_stays_linear() -> None:
    """Guards the search loop against a hang or quadratic re-scan.

    Every rejected candidate re-searches from its own second token, so text made
    entirely of rejected candidates is the worst case for the loop. ``pos``
    strictly increases on every iteration, which bounds it at one iteration per
    token; if that ever stops holding this test hangs rather than fails, which
    is the louder of the two failures.
    """
    for unit in ("Tax Court ", "Tax Zzz ", "Income Tax\nJohn Smith\n"):
        small = _best_span_seconds(unit, 2_000)
        assert small < 0.5, (unit, small)
        large = _best_span_seconds(unit, 8_000)
        # Sixteen would be quadratic. The 10 ms term absorbs scheduler noise.
        assert large < 8 * small + 0.010, (unit, small, large)


def test_a_two_token_candidate_holding_a_statutory_word_still_drops() -> None:
    """Deliberate: dropping the statutory half leaves one token, which is not a name."""
    assert patterns.person_names("Tax Smith reconciled the account") == set()
    assert patterns.person_names("the Superannuation Act applies") == set()


def test_accented_names_are_candidates() -> None:
    """Latin-1 accented letters are ordinary in Australian client data."""
    for text, name in (
        ("Zo\u00eb Nguyen filed the return", "Zo\u00eb Nguyen"),
        ("Jos\u00e9 Ram\u00edrez signed", "Jos\u00e9 Ram\u00edrez"),
        ("\u00c9mile Zola attended", "\u00c9mile Zola"),
        ("S\u00f8ren Kierkegaard called", "S\u00f8ren Kierkegaard"),
    ):
        assert name in patterns.person_names(text), text


def test_person_name_spans_point_at_the_text_they_report() -> None:
    text = "the Commissioner Anna Petrov letter"
    spans = patterns.person_name_spans(text)
    assert [text[start:end] for start, end, _value in spans] == [value for *_, value in spans]


def test_person_name_spans_are_ordered_and_report_each_person_once() -> None:
    """The retry resumes inside a rejected candidate, so a span could repeat.

    A candidate that keeps a window must resume past the whole candidate. If it
    resumed at its second token instead, "Commissioner Anna Petrov" would report
    "Anna Petrov" from the window and again from the next candidate. Only the
    span list shows it: ``person_names`` is a set and the residual sweep keeps
    one report per name per line, so both would hide the duplicate.
    """
    for text in (
        "the Commissioner Anna Petrov letter",
        "reviewed by Priya Sharma Superannuation",
        "## Income Tax\nJohn Smith prepared the return",
        "the Board Daniel Okafor and Mary Jones met",
    ):
        spans = patterns.person_name_spans(text)
        assert len(spans) == len(set(spans)), (text, spans)
        previous_end = 0
        for start, end, _value in spans:
            assert start >= previous_end, (text, spans)
            previous_end = end
