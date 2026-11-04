//! `csv.DictReader(io.StringIO(text), strict=True)` and `csv.DictWriter` with
//! the default excel dialect, ported from CPython's `_csv.c`. `io.StringIO`
//! splits lines only after `\n`, so a lone `\r` stays inside a line.

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

/// `_csv.Error` with CPython's message.
#[derive(Debug, PartialEq, Eq)]
pub struct CsvError(pub String);

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

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
            return Err(CsvError(format!("field larger than field limit ({FIELD_LIMIT})")));
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

    /// `parse_process_char` for one character, or `None` at the end of a line.
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
                Some(_) => Err(CsvError(
                    "new-line character seen in unquoted field - do you need to open the file with newline=''?".into(),
                )),
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
            Some(_) => Err(CsvError("',' expected after '\"'".into())),
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
                    // strict=True refuses a record cut off at the end of the input.
                    return Some(Err(CsvError("unexpected end of data".into())));
                }
                return None;
            };
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

/// One `csv.DictReader` row: values in header order, `None` where the row
/// was short (restval), and whether it had more fields than the header.
pub struct Row {
    pub values: Vec<Option<String>>,
    pub has_extra: bool,
}

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

/// One `csv.writer` row with QUOTE_MINIMAL and the `\r\n` line terminator.
pub fn write_row(fields: &[String]) -> String {
    let mut line = String::new();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            line.push(',');
        }
        if field.contains([',', '"', '\r', '\n']) {
            line.push('"');
            line.push_str(&field.replace('"', "\"\""));
            line.push('"');
        } else {
            line.push_str(field);
        }
    }
    if line.is_empty() && !fields.is_empty() {
        // A record of one empty field is written quoted, so it is not a blank line.
        line.push_str("\"\"");
    }
    line.push_str("\r\n");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(text: &str) -> Vec<Result<Vec<String>, CsvError>> {
        Reader::new(text).collect()
    }

    #[test]
    fn strict_reader_matches_cpython() {
        let ok = |rows: &[&[&str]]| -> Vec<Result<Vec<String>, CsvError>> {
            rows.iter()
                .map(|r| Ok(r.iter().map(|s| s.to_string()).collect()))
                .collect()
        };
        assert_eq!(records("a,b\r\n1,2\n"), ok(&[&["a", "b"], &["1", "2"]]));
        assert_eq!(records("a\n\nb"), ok(&[&["a"], &[], &["b"]]));
        assert_eq!(
            records("\"ab\"c,d"),
            vec![Err(CsvError("',' expected after '\"'".into()))]
        );
        assert_eq!(records("\"open"), vec![Err(CsvError("unexpected end of data".into()))]);
        assert!(
            records("a\rb\n")[0]
                .as_ref()
                .unwrap_err()
                .0
                .starts_with("new-line character seen")
        );
        assert_eq!(records("\"a\rb\"\n"), ok(&[&["a\rb"]]));
    }

    #[test]
    fn writer_quotes_minimally() {
        let row = |fields: &[&str]| write_row(&fields.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(
            row(&["a", "b,c", "d\"e", "f\ng", " h "]),
            "a,\"b,c\",\"d\"\"e\",\"f\ng\", h \r\n"
        );
        assert_eq!(row(&[""]), "\"\"\r\n");
        assert_eq!(row(&["", ""]), ",\r\n");
    }
}
