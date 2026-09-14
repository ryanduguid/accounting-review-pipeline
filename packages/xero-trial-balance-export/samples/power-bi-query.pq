// Load a xero-trial-balance-export CSV into Power BI with the exported schema
// intact.
//
// To use it: Power BI Desktop, Home > Transform data > New Source > Blank
// Query, then Home > Advanced Editor, and replace the whole window with this.
//
// Nothing here contacts Xero. There is no client id, no token and no ledger
// access in this query, so pointing it at the committed fabricated sample
// exercises the entire load path with no Xero connection at all.

let
    // SET THIS before running. Point it at your own export, or at the
    // fabricated sample committed in this repository to try it offline:
    // "C:\path\to\xero-trial-balance-export\samples\sample-output.csv".
    SourcePath = "C:\CHANGE-ME\tb-latest.csv",

    // The exporter's column contract, in order. export_tb.py refuses to write
    // a reshaped report, so a mismatch here means the file did not come from
    // this tool and should not be loaded blind.
    ExpectedColumns = {
        "ReportDate",
        "Tenant",
        "Section",
        "AccountID",
        "AccountName",
        "AccountCode",
        "Debit",
        "Credit",
        "YTDDebit",
        "YTDCredit"
    },

    // Read several columns wider than the contract, on purpose.
    //
    // Csv.Document normally drops fields beyond the requested column count
    // and pads shorter rows. Asking for exactly 10 without ExtraValues.Error
    // would therefore reshape a malformed file into the expected shape before
    // any check could see it, and an 11-column export would sail through the
    // header comparison below. Reading wider leaves the evidence intact: in a
    // well-formed file every column past the tenth holds no value in any row,
    // so any value there proves the file is wider than the 10 columns
    // export_tb.py writes.
    //
    // Reading only one extra column was not enough. A row whose eleventh field
    // is empty and whose twelfth carries data reads as absent in column 11,
    // and the twelfth has already been dropped by the reader, so the file
    // loaded as if it were well formed. ProbeMargin below is how many columns
    // past the contract are read and checked.
    //
    // ExtraValues.Error rejects rows beyond the probe width, including rows
    // whose first surplus value follows several empty fields. The parser
    // handles quoted commas and line breaks with its default QuoteStyle.Csv.
    // Pass ExtraValues.Error as the fourth argument: Excel 16.0 build 20430
    // ignores it in the options record.
    //
    // What "no value" looks like is not the same in every host. Power BI
    // Desktop pads a short record with null; Excel 16.0's engine pads it with
    // empty text. Removing nulls alone therefore refused an ordinary
    // ten-column file in Excel, because "" survived the removal and read as a
    // value past column 10. Both forms of padding mean the record had no such
    // field, so IsAbsent below treats both as absent and neither is mistaken
    // for data.
    //
    // Encoding 65001 is UTF-8. The exporter writes a BOM so Excel decodes
    // non-ASCII account names on a double-click open; Power Query strips it.
    ProbeMargin = 8,
    OverflowColumns = List.Transform(
        {List.Count(ExpectedColumns) + 1 .. List.Count(ExpectedColumns) + ProbeMargin},
        (index) => "Column" & Text.From(index)
    ),
    // Force parsing before removing overflow columns, so projection cannot
    // discard the parser's excess-field error.
    Probe = Table.Buffer(Csv.Document(
        File.Contents(SourcePath),
        List.Count(ExpectedColumns) + ProbeMargin,
        ",",
        ExtraValues.Error,
        65001
    )),
    IsAbsent = (value as any) as logical => value = null or value = "",
    Present = (values as list) as list => List.Select(values, (value) => not IsAbsent(value)),
    Overflow = List.Combine(
        List.Transform(OverflowColumns, (name) => Present(Table.Column(Probe, name)))
    ),
    Narrowed =
        if List.IsEmpty(Overflow) then
            Table.RemoveColumns(Probe, OverflowColumns)
        else
            error Error.Record(
                "Unexpected trial balance schema",
                "This file carries more than the ten columns export_tb.py writes.",
                "First value found past column "
                    & Text.From(List.Count(ExpectedColumns))
                    & ": "
                    & Text.From(List.First(Overflow))
            ),
    Promoted = Table.PromoteHeaders(Narrowed, [PromoteAllScalars = true]),

    // Fail closed rather than load a half-recognised report. This mirrors the
    // exporter's own balance and column checks: a reshaped file stops here
    // instead of reaching a refresh pipeline.
    Checked =
        if Table.ColumnNames(Promoted) = ExpectedColumns then
            Promoted
        else
            error Error.Record(
                "Unexpected trial balance schema",
                "This file does not have the ten columns export_tb.py writes.",
                "Expected: "
                    & Text.Combine(ExpectedColumns, ", ")
                    & " | Got: "
                    & Text.Combine(Table.ColumnNames(Promoted), ", ")
            ),

    // A truncated row is the other half of the same problem: Csv.Document
    // padded it to reach the column count, so the header check above cannot
    // see it. The exporter writes every column on every row and never leaves
    // one empty, so an absent field here means a damaged file. Absence is
    // read through IsAbsent for the same host reason as the overflow column:
    // a check for null alone misses every short row in Excel's engine.
    // Wholly blank lines are ignored rather than rejected, since a trailing
    // newline is not a defect.
    Populated = Table.SelectRows(
        Checked,
        each not List.IsEmpty(Present(Record.FieldValues(_)))
    ),
    Gaps = Table.SelectRows(
        Populated,
        each List.Count(Present(Record.FieldValues(_))) <> List.Count(ExpectedColumns)
    ),
    Complete =
        if Table.IsEmpty(Gaps) then
            Populated
        else
            error Error.Record(
                "Incomplete trial balance row",
                "A row does not carry all ten fields export_tb.py writes.",
                Text.From(Table.RowCount(Gaps)) & " row(s) are missing at least one field."
            ),

    // AccountCode stays text on purpose. Xero account codes such as 090 are
    // strings, and a numeric type silently drops the leading zero, which
    // breaks both slicers and any join back to the ledger. AccountID is a
    // GUID, also text.
    //
    // The 4 money columns use Currency.Type, a fixed decimal, rather than
    // type number: totals of a floating-point type drift, and a trial balance
    // that no longer sums to zero is worse than useless.
    Typed = Table.TransformColumnTypes(
        Complete,
        {
            {"ReportDate", type date},
            {"Tenant", type text},
            {"Section", type text},
            {"AccountID", type text},
            {"AccountName", type text},
            {"AccountCode", type text},
            {"Debit", Currency.Type},
            {"Credit", Currency.Type},
            {"YTDDebit", Currency.Type},
            {"YTDCredit", Currency.Type}
        },
        "en-AU"
    )
in
    Typed
