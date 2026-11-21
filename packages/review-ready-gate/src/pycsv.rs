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
}

/// `_csv.Error`; the gateway reports every one the same way.
#[derive(Debug, PartialEq, Eq)]
pub struct CsvError;

/// The `csv.reader` record iterator.
pub struct Reader<'a> {
    remaining: &'a str,
    line_num: usize,
}

impl<'a> Reader<'a> {
    pub fn new(text: &'a str) -> Reader<'a> {
        Reader {
            remaining: text,
            line_num: 0,
        }
    }

    /// Read a physical line and its terminator as `StringIO(newline="")` does.
    fn next_line(&mut self) -> Option<(&'a str, &'a str)> {
        if self.remaining.is_empty() {
            return None;
        }
        let bytes = self.remaining.as_bytes();
        let line_end = bytes
            .iter()
            .position(|&byte| matches!(byte, b'\r' | b'\n'))
            .unwrap_or(bytes.len());
        let (body, rest) = self.remaining.split_at(line_end);
        let width = if rest.starts_with("\r\n") {
            2
        } else {
            usize::from(!rest.is_empty())
        };
        let (ending, remaining) = rest.split_at(width);
        self.remaining = remaining;
        self.line_num += 1;
        Some((body, ending))
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

    /// Process one character within a physical line.
    fn step(&mut self, c: char) -> Result<(), CsvError> {
        match self.state {
            State::StartRecord => {
                self.state = State::StartField;
                self.unquoted(c)
            }
            State::StartField | State::InField => self.unquoted(c),
            State::InQuotedField => match c {
                '"' => {
                    self.state = State::QuoteInQuotedField;
                    Ok(())
                }
                ch => self.add_char(ch),
            },
            State::QuoteInQuotedField => self.after_quote(c),
        }
    }

    fn unquoted(&mut self, c: char) -> Result<(), CsvError> {
        match c {
            '"' if self.state == State::StartField => {
                self.state = State::InQuotedField;
                Ok(())
            }
            ',' => {
                self.save_field();
                self.state = State::StartField;
                Ok(())
            }
            ch => {
                self.state = State::InField;
                self.add_char(ch)
            }
        }
    }

    /// After a quote inside a quoted field; strict off keeps any other character.
    fn after_quote(&mut self, c: char) -> Result<(), CsvError> {
        match c {
            '"' => {
                self.state = State::InQuotedField;
                self.add_char('"')
            }
            ',' => {
                self.save_field();
                self.state = State::StartField;
                Ok(())
            }
            ch => {
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
        while let Some((body, ending)) = self.next_line() {
            for c in body.chars() {
                if record.step(c).is_err() {
                    return Some(Err(CsvError));
                }
            }
            if record.state != State::InQuotedField {
                if record.state != State::StartRecord {
                    record.save_field();
                }
                return Some(Ok(record.fields));
            }
            for c in ending.chars() {
                if record.add_char(c).is_err() {
                    return Some(Err(CsvError));
                }
            }
        }
        if record.state == State::InQuotedField {
            record.save_field();
            return Some(Ok(record.fields));
        }
        None
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
    let fields: Vec<_> = values
        .iter()
        .map(|value| {
            if value.contains([',', '"', '\r', '\n']) || (values.len() == 1 && value.is_empty()) {
                format!("\"{}\"", value.replace('"', "\"\""))
            } else {
                (*value).to_string()
            }
        })
        .collect();
    fields.join(",") + "\r\n"
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
    fn physical_endings_preserve_quoted_text_and_line_numbers() {
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
    }

    #[test]
    fn a_quoted_line_ending_counts_towards_the_field_limit() {
        let text = format!("\"{}\r\nnext,record\n", "x".repeat(FIELD_LIMIT - 1));
        let mut reader = Reader::new(&text);
        assert_eq!(reader.next(), Some(Err(CsvError)));
        assert_eq!(reader.line_num, 1);
        assert_eq!(
            reader.next(),
            Some(Ok(vec!["next".into(), "record".into()]))
        );
        assert_eq!(reader.line_num, 2);
        assert!(reader.next().is_none());
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
}
