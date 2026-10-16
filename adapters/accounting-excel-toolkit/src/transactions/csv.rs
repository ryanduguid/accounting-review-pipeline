//! The permissive Power BI CSV reader adapted to default StringIO's LF splitting.
//! LF output quoting is reused from Monthly Close's `write_record_lf`.

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
pub struct CsvError(pub String);

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// The `csv.reader` record iterator.
pub struct Reader<'a> {
    lines: std::vec::IntoIter<&'a str>,
    line_num: usize,
}

impl<'a> Reader<'a> {
    pub fn new(text: &'a str) -> Reader<'a> {
        Reader {
            lines: lines(text).into_iter(),
            line_num: 0,
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
            return Err(CsvError(format!(
                "field larger than field limit ({FIELD_LIMIT})"
            )));
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
                Some(_) => Err(CsvError("new-line character seen in unquoted field - do you need to open the file with newline=''?".into())),
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

pub fn write_record(values: &[&str]) -> String {
    let fields: Vec<_> = values
        .iter()
        .map(|value| {
            if value.contains([',', '"', '\r', '\n']) || (values.len() == 1 && value.is_empty()) {
                format!("\"{}\"", value.replace('"', "\"\""))
            } else {
                (*value).to_owned()
            }
        })
        .collect();
    fields.join(",") + "\n"
}
