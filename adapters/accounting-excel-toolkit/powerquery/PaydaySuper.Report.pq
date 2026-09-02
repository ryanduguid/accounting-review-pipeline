/*
PaydaySuper.Report

Imports the fixed 18-column CSV contract emitted by payday-super-checker.
It is deliberately an adapter, not an assessment: source verdicts, caveats,
notes, unassessable_between and producer amounts remain authoritative.  The
producer's full-width terminal record has employee_id = NOTE, provenance in
notes and blank values in every other contract field.  Only that shape is
excluded from data; its notes field is exposed as table metadata
[PaydaySuperProvenance].

All fixtures in this repository are fabricated.  Use a real report only from
an approved location outside this repository.
*/
let
    PaydaySuperReport = (FilePath as text) as table =>
    let
        RequiredHeaders = {
            "row", "employee_id", "qe_day", "pathway", "due_date", "verdict",
            "days_late", "lateness_measured_to", "sg_amount", "final_shortfall",
            "notional_earnings", "uplift_best_case", "uplift_worst_case",
            "sgc_estimate_low", "sgc_estimate_high", "caveats", "notes",
            "unassessable_between"
        },
        AmountHeaders = {
            "sg_amount", "final_shortfall", "notional_earnings",
            "uplift_best_case", "uplift_worst_case", "sgc_estimate_low",
            "sgc_estimate_high"
        },
        TextHeaders = List.Difference(RequiredHeaders, AmountHeaders),
        // Read one stable snapshot. Width validation and Csv.Document must see
        // identical bytes even if the producer replaces the file mid-refresh.
        fileBinary = Binary.Buffer(File.Contents(FilePath)),
        CsvText = Text.TrimStart(
            Text.FromBinary(fileBinary, TextEncoding.Utf8),
            {Character.FromNumber(65279)}
        ),
        CharacterCount = Text.Length(CsvText),
        QuoteCharacter = Character.FromNumber(34),
        // Frame complete records with the Mashup engine's CSV quote rules.
        // This preserves embedded newlines and avoids a state chain whose
        // evaluation depth grows with the file's character count.
        CsvRecords = if CharacterCount = 0 then
            {}
            else Lines.FromText(CsvText, QuoteStyle.Csv, false),
        RecordTable = Table.AddIndexColumn(
            Table.FromList(
                CsvRecords,
                Splitter.SplitByNothing(),
                {"RecordText"},
                null,
                ExtraValues.Error
            ),
            "RecordNumber",
            1,
            1,
            Int64.Type
        ),
        MeasureRecord = (recordText as text, recordNumber as number) as record =>
            let
                quoteCount = Text.Length(recordText) -
                    Text.Length(Text.Replace(recordText, QuoteCharacter, "")),
                // Keep the parity error outside the parser try. Otherwise M's
                // lazy evaluation lets that try catch and relabel this specific
                // EOF contract failure as generic invalid CSV.
                parsedAttempt = if Number.Mod(quoteCount, 2) <> 0 then
                    error Error.Record(
                        "PaydaySuper.Report",
                        "Malformed report row",
                        "CSV record " & Text.From(recordNumber) &
                            " ends inside an unterminated quoted field."
                    )
                    else try Csv.Document(
                        Text.ToBinary(recordText, TextEncoding.Utf8),
                        [
                            Delimiter = ",",
                            Encoding = 65001,
                            QuoteStyle = QuoteStyle.Csv,
                            ExtraValues = ExtraValues.Error
                        ]
                    ),
                parsed = if parsedAttempt[HasError] then
                    error Error.Record(
                        "PaydaySuper.Report",
                        "Malformed report row",
                        "CSV record " & Text.From(recordNumber) & " is not valid CSV."
                    )
                    else parsedAttempt[Value],
                rowCount = Table.RowCount(parsed),
                width = if rowCount = 1 then Table.ColumnCount(parsed) else
                    error Error.Record(
                        "PaydaySuper.Report",
                        "Malformed report row",
                        "CSV record " & Text.From(recordNumber) &
                            " did not parse as exactly one record."
                    )
            in
                [RecordNumber = recordNumber, Width = width],
        HeaderRecord = if
            CharacterCount = 0 or Table.IsEmpty(RecordTable)
        then
            error Error.Record(
                "PaydaySuper.Report",
                "Missing required header",
                "The report has no header row."
            )
            else RecordTable{0},
        HeaderWidth = MeasureRecord(
            HeaderRecord[RecordText],
            HeaderRecord[RecordNumber]
        )[Width],
        MeasuredRecords = Table.AddColumn(
            RecordTable,
            "Width",
            each MeasureRecord([RecordText], [RecordNumber])[Width],
            Int64.Type
        ),
        FirstMalformedRecord = Table.FirstN(
            Table.SelectRows(
                Table.Skip(MeasuredRecords, 1),
                each [Width] <> HeaderWidth
            ),
            1
        ),
        RecordWidthsChecked = if Table.IsEmpty(FirstMalformedRecord) then
            HeaderWidth
            else
                let malformed = FirstMalformedRecord{0} in
                    error Error.Record(
                        "PaydaySuper.Report",
                        "Malformed report row",
                        "CSV record " & Text.From(malformed[RecordNumber]) & " contains " &
                            Text.From(malformed[Width]) & " fields; expected exactly " &
                            Text.From(HeaderWidth) & "."
                    ),
        Shaped = Csv.Document(
            fileBinary,
            [
                Delimiter = ",",
                Columns = RecordWidthsChecked,
                Encoding = 65001,
                QuoteStyle = QuoteStyle.Csv,
                ExtraValues = ExtraValues.Error
            ]
        ),
        HeaderNames = if Table.RowCount(Shaped) = 0 then
            error Error.Record("PaydaySuper.Report", "Missing required header", "The report has no header row.")
            else List.Transform(
                Record.FieldValues(Shaped{0}),
                each if _ = null then "" else Text.From(_)
            ),
        DuplicateHeaders = List.Select(
            List.Distinct(HeaderNames),
            (header as text) as logical =>
                List.Count(List.Select(HeaderNames, each _ = header)) > 1
        ),
        DuplicateChecked = if List.IsEmpty(DuplicateHeaders) then Shaped else
            error Error.Record(
                "PaydaySuper.Report",
                "Duplicate header",
                "Duplicate header names are not valid: " & Text.Combine(DuplicateHeaders, ", ")
            ),
        Promoted = Table.PromoteHeaders(DuplicateChecked, [PromoteAllScalars = true]),
        MissingHeaders = List.Difference(RequiredHeaders, Table.ColumnNames(Promoted)),
        HeadersChecked = if List.IsEmpty(MissingHeaders) then Promoted else
            error Error.Record(
                "PaydaySuper.Report",
                "Missing required header",
                "Required header names are missing or renamed: " & Text.Combine(MissingHeaders, ", ")
            ),
        // Ignore producer additions only after all named contract fields exist.
        Projected = Table.SelectColumns(HeadersChecked, RequiredHeaders, MissingField.Error),
        Indexed = Table.AddIndexColumn(Projected, "__source_index", 0, 1, Int64.Type),
        TerminalBlankFields = List.RemoveItems(RequiredHeaders, {"employee_id", "notes"}),
        IsBlank = (value as any) as logical =>
            value = null or Text.Trim(Text.From(value)) = "",
        IsTerminalNoteCandidate = (current as record) as logical =>
            current[employee_id] = "NOTE" and
                List.AllTrue(List.Transform(
                    TerminalBlankFields,
                    (column as text) as logical => IsBlank(Record.Field(current, column))
                )),
        NoteRows = Table.SelectRows(Indexed, each IsTerminalNoteCandidate(_)),
        NoteCount = Table.RowCount(NoteRows),
        TerminalNote = if NoteCount = 0 then
            error Error.Record(
                "PaydaySuper.Report",
                "Missing terminal NOTE provenance",
                "The producer report must end with one full-shape employee_id = NOTE provenance record."
            )
            else if NoteCount <> 1 or NoteRows{0}[__source_index] <> Table.RowCount(Indexed) - 1 then
            error Error.Record(
                "PaydaySuper.Report",
                "Malformed report row",
                "The full-shape employee_id = NOTE provenance record must occur exactly once and be terminal."
            )
            else NoteRows{0},
        Provenance = TerminalNote[notes],
        ProvenanceChecked = if Provenance = null or Text.Trim(Text.From(Provenance)) = "" then
            error Error.Record(
                "PaydaySuper.Report",
                "Missing terminal NOTE provenance",
                "The terminal NOTE row must provide provenance in its notes field."
            )
            else Text.From(Provenance),
        DataRows = Table.SelectRows(Indexed, each not IsTerminalNoteCandidate(_)),
        DataRowCount = Table.RowCount(DataRows),
        ContributionRowCount = if DataRowCount = 0 then
            error Error.Record(
                "PaydaySuper.Report",
                "Missing contribution rows",
                "The producer report must contain at least one contribution row."
            )
            else DataRowCount,
        DataPresent = if ContributionRowCount > 0 then DataRows else
            error Error.Record(
                "PaydaySuper.Report",
                "Missing contribution rows",
                "Contribution validation produced an impossible state."
            ),
        MandatoryDataFields = {"row", "employee_id", "qe_day", "pathway", "verdict"},
        MalformedRows = Table.SelectRows(
            DataPresent,
            each
                let current = _ in
                    List.AnyTrue(List.Transform(
                        MandatoryDataFields,
                        (column as text) as logical =>
                            let value = Record.Field(current, column) in
                                value = null or Text.Trim(Text.From(value)) = ""
                    ))
        ),
        DataChecked = if Table.RowCount(MalformedRows) = 0 then DataPresent else
            error Error.Record(
                "PaydaySuper.Report",
                "Malformed report row",
                "Each contribution row requires row, employee_id, qe_day, pathway and verdict."
            ),
        TextTyped = Table.TransformColumns(
            DataChecked,
            List.Transform(
                TextHeaders,
                (column as text) => {column, each if _ = null then null else Text.From(_), type nullable text}
            )
        ),
        ParseAmount = (value as nullable any, column as text) as nullable number =>
            let
                raw = if value = null then "" else Text.Trim(Text.From(value)),
                parsed = if raw = "" then [HasError = false, Value = null] else
                    try Number.FromText(raw, "en-AU")
            in
                if parsed[HasError] then
                    error Error.Record(
                        "PaydaySuper.Report",
                        "Invalid amount",
                        "Column " & column & " must be blank or an en-AU number."
                    )
                else parsed[Value],
        AmountTyped = List.Accumulate(
            AmountHeaders,
            TextTyped,
            (state as table, column as text) as table =>
                Table.TransformColumns(
                    state,
                    {{column, each ParseAmount(_, column), type nullable number}}
                )
        ),
        // Force producer amount validation before a caller can count or join
        // the table without touching each amount column.
        Buffered = Table.Buffer(AmountTyped),
        Result = Table.RemoveColumns(Buffered, {"__source_index"}),
        // Metadata record fields are lazy in M. Make ordinary table use depend
        // on the provenance text so absent or malformed provenance cannot be
        // observed only when a caller happens to inspect Value.Metadata.
        ValidatedResult = if
            Text.Length(ProvenanceChecked) >= 0 and ContributionRowCount > 0
        then Result else
            error Error.Record(
                "PaydaySuper.Report",
                "Missing terminal NOTE provenance",
                "Provenance validation produced an impossible state."
            )
    in
        Value.ReplaceMetadata(ValidatedResult, [PaydaySuperProvenance = ProvenanceChecked])
in
    PaydaySuperReport
