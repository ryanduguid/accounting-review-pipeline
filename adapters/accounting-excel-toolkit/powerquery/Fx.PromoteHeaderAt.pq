// Fx.PromoteHeaderAt
// Generic fix for the ledger-export pattern: real column headers sit several
// rows down, below report title / entity / date metadata rows. Finds the first
// row whose first cell equals FirstHeaderValue, drops everything above it, and
// promotes it to headers.
//
// Works for Xero, MYOB and most bank exports where the header row is
// identifiable by its first cell ("Account", "Contact", "Date", ...).
//
// CAUTION when the source is Csv.Document: pass an explicit Columns count
// (e.g. [Delimiter=",", Columns=8]). Csv.Document infers width from the
// FIRST row, and a one-field title row collapses the whole file to one
// column before this function ever sees it.
//
// Usage: Fx.PromoteHeaderAt(rawCsvTable, "Contact")
let
    PromoteHeaderAt = (Raw as table, FirstHeaderValue as text) as table =>
        let
            ColumnNames = Table.ColumnNames(Raw),
            // Compare the caller's value the same way the cells are compared,
            // or a header pasted with a trailing space passes the blank guard
            // and then matches nothing on a perfectly good export.
            Wanted = Text.Trim(FirstHeaderValue),
            // Checked feeds Table.AddIndexColumn so both guards are forced on
            // the eager path. Bound inside the row predicate instead, M's lazy
            // let-bindings would never evaluate them for a table with no rows.
            Checked =
                if List.IsEmpty(ColumnNames) then
                    error Error.Record(
                        "Fx.PromoteHeaderAt",
                        "Input table has no columns",
                        "Supply a table with at least one column before locating a header row."
                    )
                else if Wanted = "" then
                    error Error.Record(
                        "Fx.PromoteHeaderAt",
                        "Header value is blank",
                        "FirstHeaderValue must identify a non-blank first-cell value."
                    )
                else
                    Raw,
            FirstColumn = ColumnNames{0},
            Indexed = Table.AddIndexColumn(Checked, "__idx", 0, 1, Int64.Type),
            Matches = Table.SelectRows(
                Indexed,
                each Text.Trim(Text.From(Record.Field(_, FirstColumn) ?? "")) = Wanted
            )[__idx],
            HeaderIdx =
                if List.IsEmpty(Matches) then
                    error Error.Record(
                        "Fx.PromoteHeaderAt",
                        "Header row not found",
                        "No row has """ & Wanted & """ as its first cell: wrong report, or the export format changed."
                    )
                else
                    List.Min(Matches),
            Sliced = Table.RemoveFirstN(Raw, HeaderIdx),
            Promoted = Table.PromoteHeaders(Sliced, [PromoteAllScalars = true])
        in
            Promoted
in
    PromoteHeaderAt
