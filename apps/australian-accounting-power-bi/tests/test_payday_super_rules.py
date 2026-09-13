"""
test_payday_super_rules.py - Asserts Australian Payday Super statutory calculation rules.
"""

from __future__ import annotations

import csv
import datetime
import sys
import unittest
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent.parent
SAMPLES_DIR = BASE_DIR / "samples"

sys.path.insert(0, str(BASE_DIR / "tools"))
from generate_fixtures import (  # noqa: E402
    ADMIN_UPLIFT_RATE,
    GIC_PROJECTED_RATE,
    GIC_PUBLISHED_RATES,
    NATIONAL_HOLIDAYS,
    gic_annual_rate,
    gic_days_in_year,
    gic_quarter,
    gic_rate_is_published,
    notional_earnings,
)

# Every whole-of-state or whole-of-territory public holiday between 1 July and 31 December
# 2026 that the documented sample calendar covers, listed here from its own source rather
# than imported from the generator: a date the generator drops must fail this suite.
#
# Picnic Day, Monday 3 August 2026, appears on https://nt.gov.au/nt-public-holidays without
# the footnote that marks the regional show days, so it applies across the whole Territory.
# WA's King's Birthday, Monday 28 September 2026, is on
# https://www.wa.gov.au/service/employment/workplace-arrangements/public-holidays-western-australia
# in the statewide table; the same page lists the 2 local government districts that
# substitute another day. Queensland's King's Birthday, Monday 5 October 2026, is on
# https://www.qld.gov.au/recreation/travel/holidays/public, which states the first-Monday-in-
# October rule; the same Monday is Labour Day in NSW
# (https://www.nsw.gov.au/about-nsw/public-holidays), South Australia
# (https://www.safework.sa.gov.au/resources/public-holidays) and the ACT
# (https://www.act.gov.au/__data/assets/pdf_file/0004/2155495/ACT-Public-Holidays-2026.pdf).
# All of them stop the clock under the methodology's whole-of-any-state-or-territory rule.
#
# Deliberately absent, and disclosed in docs/compliance-methodology.md: Victoria's Friday
# before the AFL Grand Final (25 September 2026) and Melbourne Cup Day (3 November 2026),
# and the part-day evening holidays South Australia and Queensland observe on 24 and 31
# December. Each omission can only make a sample due date earlier, never later.
INDEPENDENT_2026_HOLIDAYS = {
    datetime.date(2026, 8, 3): "Picnic Day (NT)",
    datetime.date(2026, 9, 28): "King's Birthday (WA)",
    datetime.date(2026, 10, 5): "King's Birthday (Qld); Labour Day (NSW, SA, ACT)",
    datetime.date(2026, 12, 25): "Christmas Day",
    datetime.date(2026, 12, 28): "Boxing Day (Monday observance)",
}

# Every due date for a payday on or before 16 December 2026 lands inside that same window.
INDEPENDENT_WINDOW = (datetime.date(2026, 7, 1), datetime.date(2026, 12, 16))


def payroll_rows() -> list[dict[str, str]]:
    with open(SAMPLES_DIR / "sample-payroll-super.csv", "r", encoding="utf-8") as f:
        return list(csv.DictReader(f))


def count_business_days(start: datetime.date, end: datetime.date) -> int:
    """Count national business days after start, up to and including end."""
    days = 0
    current = start
    while current < end:
        current += datetime.timedelta(days=1)
        if current.weekday() < 5 and current not in NATIONAL_HOLIDAYS:
            days += 1
    return days


class TestPaydaySuperRules(unittest.TestCase):
    def test_sg_statutory_rate_is_12_percent(self) -> None:
        """Payday Super SG rate must be 12.0% (in force from 1 July 2025 onwards)."""
        payroll_path = SAMPLES_DIR / "sample-payroll-super.csv"
        with open(payroll_path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            for row in reader:
                q_earnings = float(row["QualifyingEarnings_CodeQ"])
                liability = float(row["SuperLiability_CodeL"])
                expected_liability = round(q_earnings * 0.12, 2)
                self.assertAlmostEqual(
                    liability,
                    expected_liability,
                    places=2,
                    msg=f"Event {row['EventID']} Super Liability ({liability}) does not match 12% of {q_earnings}",
                )

    def test_statutory_due_date_post_1_july_2026(self) -> None:
        """Under the Payday Super regime (from 1 July 2026), the due date is exactly 7 national business days after payday."""
        checked = 0
        for row in payroll_rows():
            pay_date = datetime.date.fromisoformat(row["PayDate"])
            due_date = datetime.date.fromisoformat(row["StatutoryDueDate"])
            if pay_date < datetime.date(2026, 7, 1):
                continue
            checked += 1
            self.assertEqual(
                count_business_days(pay_date, due_date),
                7,
                f"Event {row['EventID']}: due date {due_date} is not 7 national business days after payday {pay_date}",
            )
            self.assertTrue(
                due_date.weekday() < 5 and due_date not in NATIONAL_HOLIDAYS,
                f"Event {row['EventID']}: due date {due_date} falls on a weekend or national public holiday",
            )
        self.assertGreater(checked, 0, "Fixture has no post-1 July 2026 pay events to check")

    def test_sample_calendar_carries_the_independently_sourced_2026_holidays(self) -> None:
        """The calendar is checked against its sources, not against itself.

        NT Picnic Day was missing while the methodology already claimed the nationwide
        holidays plus Picnic Day, so a 29 July 2026 payday fell due on 7 August rather than
        10 August. Every other test here counts business days with the generator's own list,
        which cannot catch a date the generator never had.
        """
        for holiday, name in sorted(INDEPENDENT_2026_HOLIDAYS.items()):
            self.assertIn(
                holiday,
                NATIONAL_HOLIDAYS,
                f"{name} on {holiday} applies across a whole state or territory and must "
                "stop the sample business-day clock",
            )

    def test_committed_due_dates_recompute_from_the_independent_calendar(self) -> None:
        """Recompute the shipped due dates without touching the generator's holiday list."""

        def due_date(payday: datetime.date) -> datetime.date:
            business_days, current = 0, payday
            while business_days < 7:
                current += datetime.timedelta(days=1)
                if current.weekday() < 5 and current not in INDEPENDENT_2026_HOLIDAYS:
                    business_days += 1
            return current

        start, end = INDEPENDENT_WINDOW
        checked = set()
        for row in payroll_rows():
            payday = datetime.date.fromisoformat(row["PayDate"])
            if not start <= payday <= end:
                continue
            checked.add(payday)
            self.assertEqual(
                datetime.date.fromisoformat(row["StatutoryDueDate"]),
                due_date(payday),
                f"Event {row['EventID']} due date disagrees with the independent calendar",
            )

        # The 3 paydays whose 7-business-day windows cross one of these holidays, and 2
        # controls whose windows cross none.
        self.assertEqual(due_date(datetime.date(2026, 7, 29)), datetime.date(2026, 8, 10))
        self.assertEqual(due_date(datetime.date(2026, 9, 23)), datetime.date(2026, 10, 6))
        self.assertEqual(due_date(datetime.date(2026, 9, 30)), datetime.date(2026, 10, 12))
        self.assertEqual(due_date(datetime.date(2026, 8, 26)), datetime.date(2026, 9, 4))
        self.assertEqual(due_date(datetime.date(2026, 9, 16)), datetime.date(2026, 9, 25))
        self.assertIn(datetime.date(2026, 7, 29), checked)
        self.assertIn(datetime.date(2026, 8, 26), checked)

    def test_gic_divisor_is_days_in_the_calendar_year(self) -> None:
        """TAA 1953 s 8AAD divides the annual rate by the days in the calendar year, leap years included."""
        self.assertEqual(gic_days_in_year(2026), 365)
        self.assertEqual(gic_days_in_year(2024), 366)
        self.assertEqual(gic_days_in_year(2100), 365, "Century years are not leap years")
        self.assertEqual(gic_days_in_year(2000), 366, "Years divisible by 400 are leap years")

    def test_sg_charge_has_the_four_payday_super_components(self) -> None:
        """SG charge = individual final shortfall + notional earnings + administrative uplift (+ nil choice loading)."""
        checked = 0
        for row in payroll_rows():
            if row["ComplianceStatus"] != "LATE_BREACH":
                continue
            checked += 1
            due_date = datetime.date.fromisoformat(row["StatutoryDueDate"])
            received = datetime.date.fromisoformat(row["FundReceiptDate"])
            base_shortfall = float(row["SuperLiability_CodeL"])

            expected_interest = notional_earnings(base_shortfall, due_date, received)
            # s 18D: the fund received the contribution before any modelled assessment, so the
            # individual final shortfall is nil and only notional earnings carry the uplift.
            final_shortfall = 0.0
            uplift = round((final_shortfall + expected_interest) * ADMIN_UPLIFT_RATE, 2)

            self.assertAlmostEqual(
                float(row["GIC_NominalInterest"]),
                expected_interest,
                places=2,
                msg=f"Event {row['EventID']} notional earnings do not match daily-compounding GIC",
            )
            self.assertAlmostEqual(
                float(row["SGC_Shortfall"]),
                round(final_shortfall + expected_interest + uplift, 2),
                places=2,
                msg=f"Event {row['EventID']} SG charge does not equal shortfall + notional earnings + uplift",
            )
        self.assertGreater(checked, 0, "Fixture has no LATE_BREACH events to check")

    def test_received_contribution_reduces_the_final_shortfall_to_nil(self) -> None:
        """s 18D: a late contribution received before assessment leaves only notional earnings and uplift.

        The published SG charge must therefore never include the base liability itself for a
        row whose fund receipt date is recorded.
        """
        checked = 0
        for row in payroll_rows():
            if row["ComplianceStatus"] != "LATE_BREACH":
                continue
            checked += 1
            base_shortfall = float(row["SuperLiability_CodeL"])
            charge = float(row["SGC_Shortfall"])
            self.assertLess(
                charge,
                base_shortfall,
                msg=(
                    f"Event {row['EventID']} charge {charge} includes the base liability "
                    f"{base_shortfall}; a received contribution offsets the final shortfall"
                ),
            )
            self.assertGreater(charge, 0.0, f"Event {row['EventID']} is late so the charge cannot be nil")
        self.assertGreater(checked, 0, "Fixture has no LATE_BREACH events to check")

    def test_a_contribution_received_by_the_due_day_is_not_offset(self) -> None:
        """The s 18D offset requires received > due, so a timely payment cannot zero a real shortfall."""
        due = datetime.date(2026, 7, 15)
        self.assertEqual(notional_earnings(1000.0, due, due), 0.0)
        self.assertEqual(notional_earnings(1000.0, due, due - datetime.timedelta(days=3)), 0.0)
        self.assertGreater(notional_earnings(1000.0, due, due + datetime.timedelta(days=1)), 0.0)

    def test_gic_rate_is_selected_per_quarter_not_per_period(self) -> None:
        """GIC resets quarterly; a published quarter uses its own rate and others are projections."""
        self.assertEqual(gic_quarter(datetime.date(2026, 7, 1)), (2026, 3))
        self.assertEqual(gic_quarter(datetime.date(2026, 9, 30)), (2026, 3))
        self.assertEqual(gic_quarter(datetime.date(2026, 10, 1)), (2026, 4))

        self.assertTrue(gic_rate_is_published(datetime.date(2026, 8, 12)))
        self.assertEqual(gic_annual_rate(datetime.date(2026, 8, 12)), GIC_PUBLISHED_RATES[(2026, 3)])

        # April-June 2027 is not a published quarter, so it must fall back to the stated projection.
        self.assertFalse(gic_rate_is_published(datetime.date(2027, 5, 20)))
        self.assertEqual(gic_annual_rate(datetime.date(2027, 5, 20)), GIC_PROJECTED_RATE)

    def test_notional_earnings_use_each_days_own_rate_across_a_quarter_boundary(self) -> None:
        """An accrual spanning 30 September must not be flattened onto one quarter's rate."""
        original = dict(GIC_PUBLISHED_RATES)
        try:
            GIC_PUBLISHED_RATES[(2026, 4)] = 0.2000  # deliberately distinct from Q3
            due = datetime.date(2026, 9, 28)
            received = datetime.date(2026, 10, 3)

            balance = 1000.0
            day = due + datetime.timedelta(days=1)
            while day <= received:
                rate = 0.1143 if day.month <= 9 else 0.2000
                balance += balance * (rate / gic_days_in_year(day.year))
                day += datetime.timedelta(days=1)
            expected = round(balance - 1000.0, 2)

            self.assertAlmostEqual(notional_earnings(1000.0, due, received), expected, places=2)

            # A single-rate model would differ, which is the defect this guards.
            flat_daily = 0.1143 / gic_days_in_year(2026)
            flat = round(1000.0 * ((1 + flat_daily) ** (received - due).days - 1), 2)
            self.assertNotAlmostEqual(expected, flat, places=2)
        finally:
            GIC_PUBLISHED_RATES.clear()
            GIC_PUBLISHED_RATES.update(original)

    def test_compliance_status_flagging(self) -> None:
        """If fund receipt date is strictly after statutory due date, status must be LATE_BREACH with non-zero SGC."""
        payroll_path = SAMPLES_DIR / "sample-payroll-super.csv"
        with open(payroll_path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            for row in reader:
                rec_date = datetime.date.fromisoformat(row["FundReceiptDate"])
                due_date = datetime.date.fromisoformat(row["StatutoryDueDate"])
                status = row["ComplianceStatus"]
                sgc = float(row["SGC_Shortfall"])

                if rec_date > due_date:
                    self.assertEqual(status, "LATE_BREACH", f"Event {row['EventID']} should be flagged LATE_BREACH")
                    self.assertGreater(sgc, 0.0, f"Late event {row['EventID']} must have non-zero SGC shortfall")
                else:
                    self.assertEqual(status, "ON_TIME", f"Event {row['EventID']} should be flagged ON_TIME")
                    self.assertEqual(sgc, 0.0, f"On-time event {row['EventID']} must have 0.0 SGC shortfall")


if __name__ == "__main__":
    unittest.main()
