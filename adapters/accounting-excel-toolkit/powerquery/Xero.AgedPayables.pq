// Xero.AgedPayables
// Parse a Xero Aged Payables Summary CSV export into a typed, verified table.
//
// Accepts the standard Xero Aged Payables Summary export shape.
// The header row is detected by finding the row starting with "Contact", "Supplier", or "Vendor".
// Metadata rows above the header (report title, entity name, as-at date, ageing basis)
// are safely measured and skipped without collapsing the table width.
//
// Key contract rules:
//   1. Supplier identifier is forced to text so leading zeros and punctuation survive.
//   2. Ageing buckets and Total columns are parsed as numbers (Currency/type
//      number) under a pinned en-AU culture, the same rule Xero.TrialBalance
//      uses. A blank cell is a genuine zero; any other cell that will not
//      parse raises an error naming the column and the value, instead of
//      loading as 0.
//   3. The trailing summary "Total" row is excluded from the returned supplier rows.
//   4. Bucket and Total columns are returned as Xero exported them. This query
//      does not tie row buckets to the exported Total, so check it yourself:
//      Number.Abs(List.Sum(row buckets) - row[Total]) < 0.005, the same
//      tolerance Xero.TrialBalance documents, because the sums are IEEE
//      doubles and an exact = can fail on a genuinely tying row.
//   5. Negative balances (supplier credit notes and prepayments) are preserved as signed numbers.
//
// Usage: Xero.AgedPayables("C:\exports\aged-payables-2026-06-30.csv")
let
    AgedPayables = (FilePath as text) as table =>
        let
            fileBinary = File.Contents(FilePath),
            lines = Lines.FromBinary(fileBinary, null, null, 65001),
            headerLines = List.Select(
                lines,
                each Text.StartsWith(Text.TrimStart(_), "Contact,")
                    or Text.StartsWith(Text.TrimStart(_), "Supplier,")
                    or Text.StartsWith(Text.TrimStart(_), "Vendor,")
                    or Text.Trim(_) = "Contact"
                    or Text.Trim(_) = "Supplier"
                    or Text.Trim(_) = "Vendor"
            ),
            headerLine =
                if List.IsEmpty(headerLines) then
                    error Error.Record(
                        "Xero.AgedPayables",
                        "Header row not found",
                        "No line has ""Contact"", ""Supplier"", or ""Vendor"" as its first field: not a Xero Aged Payables export, or the format changed."
                    )
                else
                    List.First(headerLines),
            firstField = Text.Trim(List.First(Text.Split(headerLine, ","))),
            columnCount = List.Count(Text.Split(headerLine, ",")),
            Raw = Csv.Document(
                fileBinary,
                [Delimiter = ",", Columns = columnCount, Encoding = 65001, QuoteStyle = QuoteStyle.Csv]
            ),
            Indexed = Table.AddIndexColumn(Raw, "__idx", 0, 1, Int64.Type),
            HeaderMatches = Table.SelectRows(
                Indexed,
                each Text.Trim(Text.From([Column1] ?? "")) = firstField
            )[__idx],
            HeaderIdx =
                if List.IsEmpty(HeaderMatches) then
                    error Error.Record(
                        "Xero.AgedPayables",
                        "Header row not found",
                        "Header line detected in raw text but not in the parsed CSV: format changed."
                    )
                else
                    List.Min(HeaderMatches),
            Sliced = Table.RemoveFirstN(Raw, HeaderIdx),
            Promoted = Table.PromoteHeaders(Sliced, [PromoteAllScalars = true]),
            cols = Table.ColumnNames(Promoted),
            supplierCol = if List.Contains(cols, "Supplier") then "Supplier" else if List.Contains(cols, "Vendor") then "Vendor" else cols{0},
            // Drop total rows and blank rows
            Filtered = Table.SelectRows(
                Promoted,
                each let val = Text.Trim(Text.From(Record.Field(_, supplierCol) ?? "")) in
                    val <> "" and Text.Lower(val) <> "total"
            ),
            // Identify amount columns (all columns except the supplier column)
            amountCols = List.Difference(cols, {supplierCol}),
            // Culture pinned to en-AU, the same rule Xero.TrialBalance's
            // parseAmount follows: an unpinned parse follows the machine
            // locale, where de-DE reads "15234.50" as 1523450. Blank cells
            // are genuine zeros; anything else that will not parse - "TBC",
            // "#REF!", or a bracketed negative such as "(150.00)" the culture
            // does not read - errors loudly, naming the column and the cell,
            // instead of becoming a 0 that ties the row's buckets to a 0
            // Total. No parsing beyond what Xero.TrialBalance already does is
            // attempted here.
            parseAmount = (columnName as text) =>
                (raw) =>
                    let t = Text.Trim(Text.From(raw ?? "")) in
                        if t = "" then 0
                        else
                            try Number.From(t, "en-AU")
                            otherwise error Error.Record(
                                "Xero.AgedPayables",
                                "Amount is not a number",
                                "Column """ & columnName & """ cell """ & t & """ won't parse as a number. Fix the export before loading."
                            ),
            // Transform types. A named parameter, not a nested each: an inner
            // each's _ shadows the outer one, so the column name the error
            // message reports would be unreachable from inside the transform.
            TransformOps = List.Transform(
                amountCols,
                (columnName as text) => {columnName, parseAmount(columnName), type number}
            ) & {{supplierCol, each Text.Trim(Text.From(_ ?? "")), type text}},
            Typed = Table.TransformColumns(Filtered, TransformOps),
            // Rename supplier column to canonical "Supplier" if needed
            Renamed = if supplierCol <> "Supplier" then Table.RenameColumns(Typed, {{supplierCol, "Supplier"}}) else Typed
        in
            Renamed
in
    AgedPayables
