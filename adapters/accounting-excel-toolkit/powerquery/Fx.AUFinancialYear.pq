// Fx.AUFinancialYear
// Australian financial year (1 July to 30 June) attributes for any date.
// Returns a record: Label ("FY2026"), StartDate, EndDate, EndYear.
//
// Date, datetime and datetimezone values are accepted: Excel hands back
// date cells as datetime unless a Changed Type step has already run. A
// datetimezone is CONVERTED to Australian Eastern Standard Time (+10:00)
// and read there, so the same instant gets the same FY on a Sydney desktop
// and on a UTC-hosted scheduled refresh: 9am +10:00 on 1 July 2026 is
// FY2027, and so is the UTC-stamped 2026-06-30T14:30:00Z a Xero API export
// carries for 12:30am on 1 July in Sydney. AEST is +10 all year and no
// Australian jurisdiction observes daylight saving across 30 June, so the
// FY boundary is never a DST boundary. Residual: an instant in the last
// two hours of 30 June in Perth (last 30 minutes in Adelaide/Darwin) is
// already 1 July in AEST and is stamped FY+1 - switch zone yourself before
// calling if the local calendar of a western or central state is the one
// that has to decide. Text is parsed as an Australian date, so "01/07/2026"
// is 1 July whatever the machine's locale; a number is read as an Excel
// serial. A null date returns a record of nulls, so a blank cell leaves a
// blank FY on that row rather than an error.
//
// Usage:
//   Fx.AUFinancialYear(#date(2026, 3, 15))[Label]   // "FY2026"
//   Fx.AUFinancialYear(#date(2026, 8, 1))[Label]    // "FY2027"
// Add an FY column to a table:
//   Table.AddColumn(t, "FY", each Fx.AUFinancialYear([Date])[Label], type text)
let
    AUFinancialYear = (d as any) as record =>
        let
            // Date.From on a datetimezone returns the date of the value's
            // LOCAL equivalent, so the host's zone decided the answer: 9am
            // +10:00 on 1 July 2026 came out FY2027 on a Sydney desktop and
            // FY2026 on a UTC-hosted scheduled refresh, and the two only
            // disagree on financial-year boundaries. Switch to a fixed +10
            // first and Australian Eastern Standard Time decides on every
            // host. RemoveZone alone would also be host-independent, but it
            // keeps the supplied wall clock, so a UTC-stamped
            // 2026-06-30T14:30:00Z - 12:30am on 1 July in Sydney, and the
            // shape the Xero API hands back - would read as 30 June and
            // land in FY2026 on the accountant's own desktop, which is the
            // wrong answer rather than merely an inconsistent one.
            // SwitchZone converts the instant and RemoveZone then yields a
            // plain datetime, so Date.From below never receives a
            // datetimezone at all and cannot reintroduce the host's zone.
            normalised =
                if Value.Is(d, type datetimezone)
                then DateTimeZone.RemoveZone(DateTimeZone.SwitchZone(d, 10))
                else d,
            asDate = Date.From(normalised, "en-AU"),
            endYear =
                if asDate = null then
                    null
                else
                    Date.Year(asDate) + (if Date.Month(asDate) >= 7 then 1 else 0),
            result =
                if endYear = null then
                    [Label = null, StartDate = null, EndDate = null, EndYear = null]
                else
                    [
                        Label = "FY" & Text.From(endYear),
                        StartDate = #date(endYear - 1, 7, 1),
                        EndDate = #date(endYear, 6, 30),
                        EndYear = endYear
                    ]
        in
            result
in
    AUFinancialYear
