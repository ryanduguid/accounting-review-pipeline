//! Payday Super charge arithmetic: general interest charge rates and notional earnings.

use std::collections::BTreeMap;

use crate::date::{Date, is_leap};
use crate::decimal::Decimal;

/// Administrative uplift for a qualifying earnings day: 60% of individual final SG
/// shortfalls plus individual notional earnings (SGAA 1992 s 19B). Reducible by 20
/// percentage points for a clean assessment history and further on voluntary
/// disclosure; no reduction is modelled here.
pub const ADMIN_UPLIFT_RATE: &str = "0.60";

/// Calendar quarter as (year, quarter) with quarter in 1..=4.
pub type Quarter = (i32, u32);

/// ATO general interest charge annual rates. GIC is reset every quarter, so an accrual
/// that crosses a quarter boundary must use each day's own rate.
#[derive(Clone, Debug)]
pub struct GicRates {
    published: BTreeMap<Quarter, Decimal>,
    projected: Decimal,
}

impl GicRates {
    /// The published rates. Quarters the ATO has not yet published carry the last
    /// published rate forward: a stated assumption, not published data. Move a rate
    /// into this table as the ATO releases it.
    pub fn published() -> GicRates {
        let published = BTreeMap::from([
            // July-September 2026, the first quarter of the Payday Super regime
            ((2026, 3), Decimal::parse("0.1143")),
            // Published by the ATO on 4 September 2026
            ((2026, 4), Decimal::parse("0.1151")),
        ]);
        let projected = published.values().next_back().expect("a published rate").clone();
        GicRates { published, projected }
    }

    /// The table with one quarter's rate replaced; the projection is unchanged.
    pub fn with_rate(mut self, quarter: Quarter, rate: Decimal) -> GicRates {
        self.published.insert(quarter, rate);
        self
    }

    pub fn projected(&self) -> &Decimal {
        &self.projected
    }

    pub fn rate_for(&self, quarter: Quarter) -> Option<&Decimal> {
        self.published.get(&quarter)
    }

    /// Annual GIC rate applying on `day`: published where the ATO has released it,
    /// else projected.
    pub fn annual_rate(&self, day: Date) -> &Decimal {
        self.published.get(&quarter(day)).unwrap_or(&self.projected)
    }

    /// True when `day` falls in a quarter whose rate the ATO has published.
    pub fn is_published(&self, day: Date) -> bool {
        self.published.contains_key(&quarter(day))
    }
}

/// Calendar quarter containing `day`.
pub fn quarter(day: Date) -> Quarter {
    (day.year(), (day.month() - 1) / 3 + 1)
}

/// Daily GIC divisor: the number of days in the calendar year (TAA 1953 s 8AAD).
pub fn days_in_year(year: i32) -> i64 {
    if is_leap(year) { 366 } else { 365 }
}

/// Individual notional earnings on the base shortfall (SGAA 1992 s 19A).
///
/// Compounds daily from the day after the due day up to and including the day the
/// fund receives the contribution. Each day uses its own quarter's rate and its own
/// calendar year's divisor.
pub fn notional_earnings(rates: &GicRates, base: &Decimal, due: Date, received: Date) -> Decimal {
    if received <= due {
        return Decimal::parse("0.0");
    }
    let mut balance = base.clone();
    let mut day = due.add_days(1);
    while day <= received {
        let daily = rates.annual_rate(day).div(&Decimal::from_int(days_in_year(day.year())));
        balance = balance.add(&balance.mul(&daily));
        day = day.add_days(1);
    }
    balance.sub(base).round(2)
}
