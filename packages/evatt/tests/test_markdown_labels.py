"""F-014 regressions use the existing fabricated checksum-invalid vectors."""

import time

import pytest
from evatt import patterns
from evatt.redact import redact
from evatt.verify import findings


@pytest.mark.parametrize('label,kind,value', [
    ('TFN', 'tfn', '123 456 783'),
    ('TFN', 'tfn', '12 345 678'),
    ('ABN', 'abn', '51 824 753 557'),
    ('ACN', 'acn', '123 456 781'),
    ('Medicare', 'medicare', '2123 45671 1'),
])
@pytest.mark.parametrize('width', [1, 3, 4, 5, 16, 256])
def test_labelled_code_runs_keep_spans_counts_and_verification(label, kind, value, width):
    fence = '`' * width
    for text in (
        f'{label}: {fence}{value}{fence}',
        f'{label}: ***{fence}{value}{fence}***',
        f'{fence}***{label}***{fence}: {value}',
        f'| ***{label}*** | {fence}{value}{fence} |',
        f'{label}: *_`__{fence}{value}{fence}__`_*',
    ):
        spans = patterns.structured_spans(text)
        assert [(kind_, text[start:end]) for start, end, kind_, _ in spans] == [(kind, value)]
        assert any(finding.kind == kind and finding.value == value for finding in findings(text, []))
        redacted, counts = redact(text, [])
        assert value not in redacted
        assert counts == {kind: 1}
        assert findings(redacted, []) == ()
    assert patterns.structured_spans(value) == []


def test_four_backtick_labelled_identifier_keeps_crlf_line_positions():
    text = 'note\r\nTFN: ***````123 456 783````***\r\n'
    redacted, counts = redact(text, [])
    assert counts == {'tfn': 1}
    assert redacted.count('\n') == 2
    assert '123 456 783' not in redacted
    assert findings(redacted, []) == ()


@pytest.mark.parametrize('label,pattern', [
    ('TFN', patterns.TFN_LABELLED),
    ('ABN', patterns.ABN_LABELLED),
    ('ACN', patterns.ACN_LABELLED),
    ('Medicare', patterns.MEDICARE_LABELLED),
])
def test_failing_markup_runs_have_bounded_growth(label, pattern):
    def duration(size):
        best = float('inf')
        for _ in range(3):
            text = label + ('*_`' * size) + ' ' * size + ':' + ('`_*' * size) + 'x'
            start = time.perf_counter()
            assert pattern.search(text) is None
            best = min(best, time.perf_counter() - start)
        return best

    small = duration(5_000)
    assert small < 0.1
    large = duration(20_000)
    assert large < small * 8 + 0.01
