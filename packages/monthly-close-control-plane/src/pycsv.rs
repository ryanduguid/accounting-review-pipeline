//! `csv.DictReader` over `io.StringIO(text, newline="")` with the default
//! excel dialect, ported from CPython's `_csv.c` reader (strict off).

const FIELD_LIMIT: usize = 131_072;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    StartRecord,
    StartField,
    InField,
    InQuotedField,
    QuoteInQuotedField,
    EatCrnl,
}

/// `_csv.Error`; the gateway reports every one the same way.
#[derive(Debug, PartialEq, Eq)]
pub enum CsvError {
    FieldLimit,
    Newline,
    AfterQuote,
    UnexpectedEnd,
}

impl std::fmt::Display for CsvError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(match self {
            Self::FieldLimit => "field larger than field limit (131072)",
            Self::Newline => "new-line character seen in unquoted field - do you need to open the file with newline=''?",
            Self::AfterQuote => "',' expected after '\"'",
            Self::UnexpectedEnd => "unexpected end of data",
        })
    }
}

/// The `csv.reader` record iterator.
pub struct Reader<'a> {
    remaining: &'a str,
    line_num: usize,
    strict: bool,
    lf_only: bool,
}

impl<'a> Reader<'a> {
    pub fn new(text: &'a str) -> Reader<'a> {
        Reader {
            remaining: text,
            line_num: 0,
            strict: false,
            lf_only: false,
        }
    }

    pub fn strict(text: &'a str) -> Self {
        Self {
            strict: true,
            ..Self::new(text)
        }
    }
    pub fn strict_lf(text: &'a str) -> Self {
        Self {
            strict: true,
            lf_only: true,
            ..Self::new(text)
        }
    }

    /// Read a physical line, retaining its terminator as `StringIO` does.
    fn next_line(&mut self) -> Option<&'a str> {
        if self.remaining.is_empty() {
            return None;
        }
        let ending = if self.lf_only {
            self.remaining.find('\n')
        } else {
            self.remaining
                .as_bytes()
                .iter()
                .position(|&byte| matches!(byte, b'\r' | b'\n'))
        };
        let end = ending.map_or(self.remaining.len(), |index| {
            let width = if self.remaining.as_bytes()[index..].starts_with(b"\r\n") {
                2
            } else {
                1
            };
            index + width
        });
        let (line, remaining) = self.remaining.split_at(end);
        self.remaining = remaining;
        Some(line)
    }
}

/// One record in progress: `_csv.c`'s `ReaderObj` state.
struct Record {
    fields: Vec<String>,
    field: String,
    field_len: usize,
    state: State,
    strict: bool,
}

impl Record {
    fn save_field(&mut self) {
        self.fields.push(std::mem::take(&mut self.field));
        self.field_len = 0;
    }

    fn add_char(&mut self, ch: char) -> Result<(), CsvError> {
        if self.field_len >= FIELD_LIMIT {
            return Err(CsvError::FieldLimit);
        }
        self.field.push(ch);
        self.field_len += 1;
        Ok(())
    }

    /// End the field at a line end (`None` is the end of the line's text).
    fn end_line(&mut self, c: Option<char>) {
        self.save_field();
        self.state = if c.is_none() {
            State::StartRecord
        } else {
            State::EatCrnl
        };
    }

    /// `parse_process_char` for one character, or `None` for the end-of-line
    /// marker _csv.c feeds after each line.
    fn step(&mut self, c: Option<char>) -> Result<(), CsvError> {
        match self.state {
            State::StartRecord => match c {
                None => Ok(()),
                Some('\n' | '\r') => {
                    self.state = State::EatCrnl;
                    Ok(())
                }
                Some(ch) => {
                    self.state = State::StartField;
                    self.unquoted(Some(ch))
                }
            },
            State::StartField | State::InField => self.unquoted(c),
            State::InQuotedField => match c {
                None => Ok(()),
                Some('"') => {
                    self.state = State::QuoteInQuotedField;
                    Ok(())
                }
                Some(ch) => self.add_char(ch),
            },
            State::QuoteInQuotedField => self.after_quote(c),
            State::EatCrnl => match c {
                Some('\n' | '\r') => Ok(()),
                None => {
                    self.state = State::StartRecord;
                    Ok(())
                }
                Some(_) => Err(CsvError::Newline),
            },
        }
    }

    fn unquoted(&mut self, c: Option<char>) -> Result<(), CsvError> {
        match c {
            None | Some('\n' | '\r') => {
                self.end_line(c);
                Ok(())
            }
            Some('"') if self.state == State::StartField => {
                self.state = State::InQuotedField;
                Ok(())
            }
            Some(',') => {
                self.save_field();
                self.state = State::StartField;
                Ok(())
            }
            Some(ch) => {
                self.state = State::InField;
                self.add_char(ch)
            }
        }
    }

    /// After a quote inside a quoted field; strict off keeps any other character.
    fn after_quote(&mut self, c: Option<char>) -> Result<(), CsvError> {
        match c {
            Some('"') => {
                self.state = State::InQuotedField;
                self.add_char('"')
            }
            Some(',') => {
                self.save_field();
                self.state = State::StartField;
                Ok(())
            }
            None | Some('\n' | '\r') => {
                self.end_line(c);
                Ok(())
            }
            Some(_) if self.strict => Err(CsvError::AfterQuote),
            Some(ch) => {
                self.state = State::InField;
                self.add_char(ch)
            }
        }
    }
}

impl Iterator for Reader<'_> {
    type Item = Result<Vec<String>, CsvError>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut record = Record {
            fields: Vec::new(),
            field: String::new(),
            field_len: 0,
            state: State::StartRecord,
            strict: self.strict,
        };
        loop {
            let Some(line) = self.next_line() else {
                if self.strict && record.state == State::InQuotedField {
                    return Some(Err(CsvError::UnexpectedEnd));
                }
                if record.field_len != 0 || record.state == State::InQuotedField {
                    record.save_field();
                    return Some(Ok(record.fields));
                }
                return None;
            };
            self.line_num += 1;
            for c in line.chars().map(Some).chain(std::iter::once(None)) {
                if let Err(error) = record.step(c) {
                    return Some(Err(error));
                }
            }
            if record.state == State::StartRecord {
                return Some(Ok(record.fields));
            }
        }
    }
}

/// One `csv.DictReader` row: header-mapped values, `None` for a missing
/// field (restval), and whether the row had more fields than the header.
pub struct Row {
    pub values: Vec<Option<String>>,
    pub has_extra: bool,
}

/// `csv.DictReader`: the header record, then rows with blank records skipped.
pub struct DictReader<'a> {
    reader: Reader<'a>,
    pub fieldnames: Option<Vec<String>>,
}

impl<'a> DictReader<'a> {
    pub fn line_num(&self) -> usize {
        self.reader.line_num
    }
    pub fn new(text: &'a str) -> Result<DictReader<'a>, CsvError> {
        let mut reader = Reader::new(text);
        let fieldnames = reader.next().transpose()?;
        Ok(DictReader { reader, fieldnames })
    }
}

/// The excel dialect's minimal quoting and CRLF record terminator.
pub fn write_record(values: &[&str]) -> String {
    write_with_ending(values, true)
}
pub fn write_record_lf(values: &[&str]) -> String {
    write_with_ending(values, false)
}
fn write_with_ending(values: &[&str], crlf: bool) -> String {
    let fields: Vec<_> = values
        .iter()
        .map(|value| {
            if value.contains([',', '"', '\n'])
                || (crlf && value.contains('\r'))
                || (values.len() == 1 && value.is_empty())
            {
                format!("\"{}\"", value.replace('"', "\"\""))
            } else {
                (*value).to_string()
            }
        })
        .collect();
    fields.join(",") + if crlf { "\r\n" } else { "\n" }
}

impl Iterator for DictReader<'_> {
    type Item = Result<Row, CsvError>;

    fn next(&mut self) -> Option<Self::Item> {
        let width = self.fieldnames.as_ref()?.len();
        loop {
            match self.reader.next()? {
                Err(error) => return Some(Err(error)),
                Ok(row) if row.is_empty() => continue,
                Ok(row) => {
                    let has_extra = row.len() > width;
                    let mut values: Vec<Option<String>> =
                        row.into_iter().take(width).map(Some).collect();
                    values.resize(width, None);
                    return Some(Ok(Row { values, has_extra }));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(text: &str) -> Vec<Result<Vec<String>, CsvError>> {
        Reader::new(text).collect()
    }

    #[test]
    fn a_line_end_before_any_character_leaves_the_record_open_and_empty() {
        // _csv.c's START_RECORD ignores the end-of-line marker of an empty line.
        let mut record = Record {
            fields: Vec::new(),
            field: String::new(),
            field_len: 0,
            state: State::StartRecord,
            strict: false,
        };
        assert_eq!(record.step(None), Ok(()));
        assert!(record.state == State::StartRecord);
        assert!(record.fields.is_empty() && record.field.is_empty());
    }

    fn ok(rows: &[&[&str]]) -> Vec<Result<Vec<String>, CsvError>> {
        rows.iter()
            .map(|r| Ok(r.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    #[test]
    fn reader_matches_cpython() {
        assert_eq!(records("a,b\r\n1,2\n"), ok(&[&["a", "b"], &["1", "2"]]));
        assert_eq!(records("a\n\nb"), ok(&[&["a"], &[], &["b"]]));
        assert_eq!(records("\"x\"\"y\",\"a\nb\"\r"), ok(&[&["x\"y", "a\nb"]]));
        assert_eq!(records("\"ab\"c,d"), ok(&[&["abc", "d"]]));
        assert_eq!(records("a\"b,c"), ok(&[&["a\"b", "c"]]));
        assert_eq!(records("\"open"), ok(&[&["open"]]));
        assert_eq!(records("a,"), ok(&[&["a", ""]]));
        assert_eq!(records("a\rb"), ok(&[&["a"], &["b"]]));
        assert_eq!(records("a\0b"), ok(&[&["a\0b"]]));
        let long = "x".repeat(FIELD_LIMIT + 1);
        assert_eq!(records(&long), vec![Err(CsvError::FieldLimit)]);
        assert_eq!(records(&"x".repeat(FIELD_LIMIT)).len(), 1);
    }

    #[test]
    fn physical_line_endings_preserve_multiline_fields_and_record_numbers() {
        let mut reader = DictReader::new("Name,Value\r\n\"First\r\nSecond\",1\rÉ,2\n").unwrap();
        assert_eq!(reader.line_num(), 1);
        let first = reader.next().unwrap().unwrap();
        assert_eq!(
            first.values,
            [Some("First\r\nSecond".into()), Some("1".into())]
        );
        assert_eq!(reader.line_num(), 3);
        let last = reader.next().unwrap().unwrap();
        assert_eq!(last.values, [Some("É".into()), Some("2".into())]);
        assert_eq!(reader.line_num(), 4);
        assert!(reader.next().is_none());

        let mut saved = Reader::strict_lf("\"First\rSecond\",1\nA\rB,2\n");
        assert_eq!(
            saved.next(),
            Some(Ok(vec!["First\rSecond".into(), "1".into()]))
        );
        assert_eq!(saved.next(), Some(Err(CsvError::Newline)));
        assert!(saved.next().is_none());
    }

    #[test]
    fn dict_reader_skips_blank_rows_and_marks_extra_fields() {
        let mut reader = DictReader::new("A,B\n\n1\n1,2,3\n").unwrap();
        assert_eq!(
            reader.fieldnames,
            Some(vec!["A".to_string(), "B".to_string()])
        );
        let first = reader.next().unwrap().unwrap();
        assert_eq!(first.values, vec![Some("1".to_string()), None]);
        assert!(!first.has_extra);
        assert!(reader.next().unwrap().unwrap().has_extra);
        assert!(reader.next().is_none());
    }

    #[test]
    fn strict_policy_reader_preserves_quoted_fields_and_refuses_repairs() {
        assert_eq!(
            Reader::strict("A,B\n\"ACCT,1\",yes\n").collect::<Vec<_>>(),
            ok(&[&["A", "B"], &["ACCT,1", "yes"]])
        );
        assert_eq!(
            Reader::strict("\"open").next(),
            Some(Err(CsvError::UnexpectedEnd))
        );
        assert_eq!(
            Reader::strict("\"ab\"c,d").next(),
            Some(Err(CsvError::AfterQuote))
        );
        assert_eq!(
            Reader::new("\"ab\"c,d").next(),
            Some(Ok(vec!["abc".into(), "d".into()]))
        );
    }
}
