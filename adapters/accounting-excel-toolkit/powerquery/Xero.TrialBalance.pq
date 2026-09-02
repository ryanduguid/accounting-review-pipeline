// Xero.TrialBalance
// Parse a Xero Trial Balance CSV export into a typed table.
//
// Accepts two CSV header shapes, told apart by the header row's first field:
//   "Account"      - combined account cell: code and name in one field.
//   "Account Code" - separate "Account Code" and "Account Name" fields.
// Both come out the same shape: AccountCode, AccountName, then the export's
// remaining columns.
//
// The separate-column shape was observed in Xero's interactive Trial Balance
// CSV export on 20 August 2026. The checked-in CSVs are fabricated fixtures,
// and their checks establish parser behaviour for these supported shapes only,
// not an official or stable Xero interactive-export schema. If Xero changes
// the interactive report/export UI, revalidate the parser before relying on it.
// Use a fresh non-client export kept outside the repository for that check.
//
// Handles three traps in Xero's export format:
//   1. Metadata rows above the real header (report title, entity, date).
//      Csv.Document infers column count from the FIRST row. A one-field
//      title row would collapse the whole file to one column, so the header
//      row is located first and its width passed in explicitly.
//   2. Two column pairs with different meanings: "Debit"/"Credit" hold the
//      current-period MOVEMENT, "YTD Debit"/"YTD Credit" hold the AS-AT
//      balances, and a full export carries both. Default takes the YTD
//      (as-at) pair when present (the balances a trial balance is loaded
//      for), else the period pair. Pass useYTD = true to require the YTD
//      pair, useYTD = false to take the period pair; either explicit
//      choice errors when the export lacks that pair.
//   3. The trailing Total row (dropped; recompute totals, don't trust them).
//
// Combined-layout cells split on the trailing " (code)" parenthetical; a
// "090 - Business Bank Account" dash prefix (Xero's account-picker display
// shape) still splits as a fallback. A code is alphanumeric, at most 10
// characters, with at least one digit ("090", "GST1"), so "Rent (Sydney)"
// keeps its parenthetical and a purely alphabetic prefix ("Interest - Bank")
// stays in the name. Codes are forced to text so leading zeros survive;
// code-less accounts (Xero bank and credit-card accounts) carry null.
//
// Usage: Xero.TrialBalance("C:\exports\entity-tb-2026-06-30-accrual.csv")
//        Xero.TrialBalance(path, false)   // period movement, not as-at
// Check after load (the sums are IEEE doubles, so tolerance, not exact =):
//   Number.Abs(List.Sum(result[Debit]) - List.Sum(result[Credit])) < 0.005
let
    TrialBalance = (FilePath as text, optional useYTD as nullable logical) as table =>
        let
            fileBinary = File.Contents(FilePath),
            // Locate the header line and measure its width BEFORE parsing,
            // so the one-field title row can't collapse the column count.
            // The header is the line whose FIRST FIELD is exactly "Account"
            // (combined layout) or "Account Code" (separate-column layout)
            // because a prefix match would be hijacked by an entity name like
            // "Accountable Plumbing Pty Ltd" on the metadata rows above.
            lines = Lines.FromBinary(fileBinary, null, null, 65001),
            headerLines = List.Select(
                lines,
                each Text.StartsWith(Text.TrimStart(_), "Account,")
                    or Text.StartsWith(Text.TrimStart(_), "Account Code,")
                    or Text.Trim(_) = "Account"
                    or Text.Trim(_) = "Account Code"
            ),
            headerLine =
                if List.IsEmpty(headerLines) then
                    error Error.Record(
                        "Xero.TrialBalance",
                        "Header row not found",
                        "No line has ""Account"" or ""Account Code"" as its first field: not a Xero TB export, or the format changed."
                    )
                else
                    List.First(headerLines),
            firstField = Text.Trim(List.First(Text.Split(headerLine, ","))),
            splitLayout = firstField = "Account Code",
            columnCount = List.Count(Text.Split(headerLine, ",")),
            Raw = Csv.Document(
                fileBinary,
                [Delimiter = ",", Columns = columnCount, Encoding = 65001, QuoteStyle = QuoteStyle.Csv]
            ),
            Indexed = Table.AddIndexColumn(Raw, "__idx", 0, 1, Int64.Type),
            // same header test as the raw-lines pass above (first cell
            // exactly the first field detected there), so both passes
            // agree on the header row
            HeaderMatches = Table.SelectRows(
                Indexed,
                each Text.Trim(Text.From([Column1] ?? "")) = firstField
            )[__idx],
            HeaderIdx =
                if List.IsEmpty(HeaderMatches) then
                    error Error.Record(
                        "Xero.TrialBalance",
                        "Header row not found",
                        "Header line detected in raw text but not in the parsed CSV: format changed."
                    )
                else
                    List.Min(HeaderMatches),
            Sliced = Table.RemoveFirstN(Raw, HeaderIdx),
            Promoted = Table.PromoteHeaders(Sliced, [PromoteAllScalars = true]),
            // Choose ONE debit/credit pair. Debit/Credit = period movement,
            // YTD Debit/YTD Credit = as-at balances. Null (the default)
            // prefers the as-at pair; explicit true/false demands its pair.
            cols = Table.ColumnNames(Promoted),
            hasPeriod = List.Contains(cols, "Debit") and List.Contains(cols, "Credit"),
            hasYTD = List.Contains(cols, "YTD Debit") and List.Contains(cols, "YTD Credit"),
            takeYTD =
                if useYTD = true then
                    if hasYTD then true
                    else error Error.Record("Xero.TrialBalance", "YTD columns not found",
                        "useYTD = true but the export has no YTD Debit/YTD Credit columns.")
                else if useYTD = false then
                    if hasPeriod then false
                    else error Error.Record("Xero.TrialBalance", "Period columns not found",
                        "useYTD = false but the export has no Debit/Credit columns.")
                else
                    hasYTD,
            Selected =
                if not hasPeriod and not hasYTD then
                    error Error.Record("Xero.TrialBalance", "Debit/Credit columns not found",
                        "Expected Debit/Credit or YTD Debit/YTD Credit after the header row.")
                else if takeYTD then
                    Table.RenameColumns(
                        Table.RemoveColumns(Promoted, {"Debit", "Credit"}, MissingField.Ignore),
                        {{"YTD Debit", "Debit"}, {"YTD Credit", "Credit"}}
                    )
                else
                    Table.RemoveColumns(Promoted, {"YTD Debit", "YTD Credit"}, MissingField.Ignore),
            // Drop blank rows and Xero's Total row. Exactly "Total" only:
            // a starts-with test would also delete a code-less account
            // named "Total Tools Trade Card". In the separate-column layout
            // the total row carries "Total" in either cell, code blank.
            Body =
                if splitLayout then
                    Table.SelectRows(
                        Selected,
                        each
                            (Text.Trim(Text.From([Account Code] ?? "")) <> ""
                                or Text.Trim(Text.From([Account Name] ?? "")) <> "")
                            and Text.Trim(Text.From([Account Code] ?? "")) <> "Total"
                            and Text.Trim(Text.From([Account Name] ?? "")) <> "Total"
                    )
                else
                    Table.SelectRows(
                        Selected,
                        each
                            Text.Trim(Text.From([Account] ?? "")) <> ""
                            and Text.Trim(Text.From([Account])) <> "Total"
                    ),
            // Culture pinned to en-AU: Xero CSVs always use "." decimal, but
            // an unpinned Number.From follows the machine locale; de-DE
            // reads "15234.50" as 1523450. Blank cells are genuine zeros;
            // anything else that won't parse ("TBC", "#REF!") errors loudly
            // instead of becoming a 0 that still balances.
            parseAmount = (raw) =>
                let t = Text.Trim(Text.From(raw ?? "")) in
                    if t = "" then 0
                    else
                        try Number.From(t, "en-AU")
                        otherwise error Error.Record(
                            "Xero.TrialBalance",
                            "Amount is not a number",
                            "Debit/Credit cell """ & t & """ won't parse as a number. Fix the export before loading."
                        ),
            Typed = Table.TransformColumns(
                Body,
                {
                    {"Debit", parseAmount, type number},
                    {"Credit", parseAmount, type number}
                }
            ),
            // Combined layout: "Business Bank Account (090)" -> code "090",
            // name "Business Bank Account"; "090 - Business Bank Account"
            // (account-picker shape) splits on " - " as a fallback. A code
            // is alphanumeric with at least one digit, max 10 chars ("090",
            // "GST1", MYOB-migration codes); rows without a code keep a
            // null code. Separate-column layout maps straight across.
            Parsed =
                if splitLayout then
                    Table.TransformColumns(
                        Table.RenameColumns(
                            Typed,
                            {{"Account Code", "AccountCode"}, {"Account Name", "AccountName"}}
                        ),
                        {{"AccountCode",
                            each let t = Text.Trim(Text.From(_ ?? "")) in if t = "" then null else t,
                            type nullable text},
                         {"AccountName",
                            each Text.Trim(Text.From(_ ?? "")),
                            type text}}
                    )
                else
                    let
                        WithCode = Table.AddColumn(
                            Typed,
                            "AccountCode",
                            each
                                let
                                    trimmed = Text.Trim([Account]),
                                    isCode = (c as nullable text) as logical =>
                                        c <> null
                                        and c <> ""
                                        and c <> trimmed
                                        and Text.Length(c) <= 10
                                        and Text.Select(c, {"0".."9"}) <> ""
                                        and Text.Select(c, {"0".."9", "A".."Z", "a".."z"}) = c,
                                    parenPos =
                                        if Text.EndsWith(trimmed, ")")
                                        then Text.PositionOf(trimmed, " (", Occurrence.Last)
                                        else -1,
                                    parenCandidate =
                                        if parenPos >= 0
                                        then Text.Range(trimmed, parenPos + 2, Text.Length(trimmed) - parenPos - 3)
                                        else null,
                                    dashCandidate =
                                        if Text.Contains(trimmed, " - ")
                                        then Text.BeforeDelimiter(trimmed, " - ")
                                        else null
                                in
                                    if isCode(parenCandidate) then parenCandidate
                                    else if isCode(dashCandidate) then dashCandidate
                                    else null,
                            type nullable text
                        ),
                        WithName = Table.AddColumn(
                            WithCode,
                            "AccountName",
                            each
                                let trimmed = Text.Trim([Account])
                                in
                                    if [AccountCode] = null then trimmed
                                    else if Text.EndsWith(trimmed, " (" & [AccountCode] & ")")
                                    then Text.Start(trimmed, Text.Length(trimmed) - Text.Length([AccountCode]) - 3)
                                    else Text.AfterDelimiter(trimmed, " - "),
                            type text
                        )
                    in
                        Table.RemoveColumns(WithName, {"Account"}),
            // Same output shape from both layouts: AccountCode and
            // AccountName up front, the export's remaining columns after.
            Result = Table.ReorderColumns(
                Parsed,
                {"AccountCode", "AccountName"}
                    & List.RemoveItems(Table.ColumnNames(Parsed), {"AccountCode", "AccountName"})
            )
        in
            Result
in
    TrialBalance
