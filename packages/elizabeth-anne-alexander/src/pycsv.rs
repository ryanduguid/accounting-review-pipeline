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
pub struct CsvError;

/// Splits like `StringIO(newline="")` iteration: after `\n`, a lone `\r`, or `\r\n`.
fn lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push(&text[start..=i]);
                start = i + 1;
            }
            b'\r' => {
                let end = if bytes.get(i + 1) == Some(&b'\n') { i + 2 } else { i + 1 };
                out.push(&text[start..end]);
                start = end;
                i = end - 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// The `csv.reader` record iterator.
pub struct Reader<'a> {
    lines: std::vec::IntoIter<&'a str>,
}

impl<'a> Reader<'a> {
    pub fn new(text: &'a str) -> Reader<'a> {
        Reader {
            lines: lines(text).into_iter(),
        }
    }
}

/// One record in progress: `_csv.c`'s `ReaderObj` state.
struct Record {
    fields: Vec<String>,
    field: String,
    field_len: usize,
    state: State,
}

impl Record {
    fn save_field(&mut self) {
        self.fields.push(std::mem::take(&mut self.field));
        self.field_len = 0;
    }

    fn add_char(&mut self, ch: char) -> Result<(), CsvError> {
        if self.field_len >= FIELD_LIMIT {
            return Err(CsvError);
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
        if self.state == State::StartRecord {
            match c {
                None => return Ok(()),
                Some('\n' | '\r') => {
                    self.state = State::EatCrnl;
                    return Ok(());
                }
                Some(_) => self.state = State::StartField,
            }
        }
        match self.state {
            State::StartRecord => unreachable!("handled above"),
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
                Some(_) => Err(CsvError),
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
        };
        loop {
            let Some(line) = self.lines.next() else {
                if record.field_len != 0 || record.state == State::InQuotedField {
                    record.save_field();
                    return Some(Ok(record.fields));
                }
                return None;
            };
            for c in line.chars().map(Some).chain(std::iter::once(None)) {
                if record.step(c).is_err() {
                    return Some(Err(CsvError));
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
    pub fn new(text: &'a str) -> Result<DictReader<'a>, CsvError> {
        let mut reader = Reader::new(text);
        let fieldnames = reader.next().transpose()?;
        Ok(DictReader { reader, fieldnames })
    }
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
                    let mut values: Vec<Option<String>> = row.into_iter().take(width).map(Some).collect();
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
        assert_eq!(records(&long), vec![Err(CsvError)]);
        assert_eq!(records(&"x".repeat(FIELD_LIMIT)).len(), 1);
    }

    #[test]
    fn dict_reader_skips_blank_rows_and_marks_extra_fields() {
        let mut reader = DictReader::new("A,B\n\n1\n1,2,3\n").unwrap();
        assert_eq!(reader.fieldnames, Some(vec!["A".to_string(), "B".to_string()]));
        let first = reader.next().unwrap().unwrap();
        assert_eq!(first.values, vec![Some("1".to_string()), None]);
        assert!(!first.has_extra);
        assert!(reader.next().unwrap().unwrap().has_extra);
        assert!(reader.next().is_none());
    }
}
