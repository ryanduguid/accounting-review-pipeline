use super::*;

struct Review<'a> {
    manifest: &'a Json,
    control: Option<&'a Json>,
    digest: String,
    buckets: Vec<(usize, String)>,
    total_index: usize,
    width: usize,
    footer_record: Option<BigInt>,
    entity: bool,
    cutoff: bool,
    rows: Vec<Json>,
    ties: Vec<Json>,
    issues: Vec<Json>,
    footer: Option<(Vec<Amount>, Amount)>,
    columns: Vec<Amount>,
    row_total: Amount,
}

fn header(records: &[Vec<String>]) -> Result<usize, String> {
    let candidates = headers(records);
    if candidates.len() != 1 {
        return Err("Expected exactly one aged-summary header.".into());
    }
    let index = candidates[0];
    let mut unique = std::collections::BTreeSet::new();
    for name in &records[index] {
        if !unique.insert(name)
            || !(name == &records[index][0]
                || name == "Current"
                || name == "Total"
                || BUCKETS.contains(&name.as_str()))
        {
            return Err("Duplicate, contradictory or unsupported summary headers.".into());
        }
    }
    Ok(index)
}

impl<'a> Review<'a> {
    fn new(
        content: &[u8],
        records: &[Vec<String>],
        header: usize,
        manifest: &'a Json,
        control: Option<&'a Json>,
    ) -> Result<Self, String> {
        let columns = &records[header];
        let buckets: Vec<_> = columns
            .iter()
            .enumerate()
            .filter(|(_, s)| s.as_str() == "Current" || BUCKETS.contains(&s.as_str()))
            .map(|(i, s)| (i, s.clone()))
            .collect();
        let mut issues = Vec::new();
        let digest = hash(content);
        let (entity, cutoff, footer_record) =
            metadata(records, header, manifest, &digest, &mut issues)?;
        let sums = vec![Amount::zero(); buckets.len()];
        Ok(Self {
            manifest,
            control,
            digest,
            buckets,
            total_index: columns.iter().position(|s| s == "Total").unwrap(),
            width: columns.len(),
            footer_record,
            entity,
            cutoff,
            rows: Vec::new(),
            ties: Vec::new(),
            issues,
            footer: None,
            columns: sums,
            row_total: Amount::zero(),
        })
    }

    fn record(&mut self, number: usize, record: &[String]) -> Result<(), String> {
        if !record.iter().any(|s| nonblank(s)) {
            return Ok(());
        }
        if record.len() != self.width {
            return Err(format!(
                "Source record {number}: field count does not match the header."
            ));
        }
        if self.footer.is_some() {
            return if record[0].trim_matches(space) == "Percentage of total" {
                Ok(())
            } else {
                Err("Unexpected record after the designated terminal footer.".into())
            };
        }
        if !nonblank(&record[0]) {
            return Err(format!("Source record {number}: missing contact label."));
        }
        self.reconcile(number, record)
    }

    fn reconcile(&mut self, number: usize, record: &[String]) -> Result<(), String> {
        let values: Vec<_> = self
            .buckets
            .iter()
            .map(|(i, _)| amount::parse(&record[*i]))
            .collect::<Result<_, _>>()?;
        let total = amount::parse(&record[self.total_index])?;
        let sum = values.iter().fold(Amount::zero(), |s, v| s.add(v));
        if self.footer_record.as_ref() == Some(&BigInt::from(number)) {
            if record[0].trim_matches(space) != "Total" {
                return Err("Designated footer does not have the Total label.".into());
            }
            tie(
                "footer buckets to total",
                &sum,
                &total,
                &mut self.ties,
                &mut self.issues,
            );
            self.footer = Some((values, total));
        } else {
            self.contact(number, record, values, total, sum);
        }
        Ok(())
    }

    fn contact(
        &mut self,
        number: usize,
        record: &[String],
        values: Vec<Amount>,
        total: Amount,
        sum: Amount,
    ) {
        if self
            .buckets
            .iter()
            .map(|(i, _)| *i)
            .chain([self.total_index])
            .all(|i| !nonblank(&record[i]))
        {
            issue(
                &mut self.issues,
                format!(
                    "Source record {number}: blank amounts cannot establish a contact row; sectioned exports need review."
                ),
            );
        }
        tie(
            &format!("record {number} buckets to total"),
            &sum,
            &total,
            &mut self.ties,
            &mut self.issues,
        );
        let mut row = Json::object([
            ("source_record", Json::Int(number.into())),
            ("Contact", Json::str(&record[0])),
        ]);
        for (i, (_, name)) in self.buckets.iter().enumerate() {
            self.columns[i] = self.columns[i].add(&values[i]);
            row.set(name, Json::str(&values[i].to_string()));
        }
        row.set("Total", Json::str(&total.to_string()));
        self.row_total = self.row_total.add(&total);
        self.rows.push(row);
    }

    fn footer(&mut self) {
        if self.rows.is_empty() {
            issue(
                &mut self.issues,
                "No contact rows supplied; completeness is unconfirmed.",
            );
        }
        if let Some((values, total)) = &self.footer {
            tie(
                "contact totals to export footer",
                &self.row_total,
                total,
                &mut self.ties,
                &mut self.issues,
            );
            for (i, (_, name)) in self.buckets.iter().enumerate() {
                tie(
                    &format!("contact column {name} to export footer {name}"),
                    &self.columns[i],
                    &values[i],
                    &mut self.ties,
                    &mut self.issues,
                );
            }
        } else {
            issue(
                &mut self.issues,
                "Supplied footer record is absent; export total is unconfirmed.",
            );
        }
    }

    fn screen(&mut self) -> Result<(), String> {
        let count = self
            .manifest
            .get("on_screen_rows")
            .and_then(Json::as_plain_int)
            .and_then(ToPrimitive::to_usize);
        if count != Some(self.rows.len()) {
            issue(
                &mut self.issues,
                "On-screen contact row count is missing or does not agree.",
            );
        }
        let on_screen = self.manifest.get("on_screen_total");
        if on_screen.is_none_or(|v| !nonblank(&json::str_value(v))) {
            issue(&mut self.issues, "On-screen report total was not checked.");
        } else if let Some((_, total)) = &self.footer {
            tie(
                "export footer to on-screen total",
                total,
                &parsed_amount(on_screen.unwrap())?,
                &mut self.ties,
                &mut self.issues,
            );
        }
        Ok(())
    }

    fn comparable(&self, control: &Json) -> bool {
        self.entity
            && self.cutoff
            && COMPARABLE.iter().all(|key| {
                field(self.manifest, key)
                    .is_some_and(|s| nonblank(&s) && control.get(key).is_some_and(|v| v.is_str(&s)))
            })
    }

    fn control(&mut self) -> Result<(), String> {
        let Some(control) = self.control else {
            issue(&mut self.issues, "Receivables control evidence is missing.");
            return Ok(());
        };
        let mut comparable = self.comparable(control);
        let sign = control
            .get("sign_convention")
            .is_some_and(|v| v.is_str("debit-positive"));
        if !comparable || !sign {
            issue(
                &mut self.issues,
                "Source or control metadata is unconfirmed or differs; control tie is unsupported.",
            );
        }
        if !field(control, "source").is_some_and(|s| nonblank(&s)) {
            issue(&mut self.issues, "Control source reference is missing.");
            comparable = false;
        }
        let value = control.get("amount");
        if value.is_none_or(|v| !nonblank(&json::str_value(v))) {
            issue(&mut self.issues, "Receivables control amount is missing.");
        } else {
            let amount = parsed_amount(value.unwrap())?;
            if comparable
                && sign
                && let Some((_, total)) = &self.footer
            {
                tie(
                    "export footer to receivables control",
                    total,
                    &amount,
                    &mut self.ties,
                    &mut self.issues,
                );
            }
        }
        Ok(())
    }

    fn finish(self) -> Json {
        Json::object([
            (
                "summary_status",
                Json::str(if self.issues.is_empty() {
                    "PASS"
                } else {
                    "REVIEW"
                }),
            ),
            ("debtor_decisions", Json::str("REVIEW")),
            ("source_sha256", Json::str(&self.digest)),
            ("manifest", self.manifest.clone()),
            ("control", self.control.cloned().unwrap_or(Json::Null)),
            ("rows", Json::Array(self.rows)),
            ("ties", Json::Array(self.ties)),
            ("exceptions", Json::Array(self.issues)),
            ("boundary", Json::str(BOUNDARY)),
        ])
    }
}

pub fn inspect_export(
    content: &[u8],
    manifest: &Json,
    control: Option<&Json>,
) -> Result<Json, String> {
    if !matches!(manifest, Json::Object(_))
        || control.is_some_and(|v| !matches!(v, Json::Object(_)))
    {
        return Err("Manifest and control must be JSON objects.".into());
    }
    let records = records(content)?;
    let header = header(&records)?;
    let mut review = Review::new(content, &records, header, manifest, control)?;
    for (index, row) in records.iter().enumerate().skip(header + 1) {
        review.record(index + 1, row)?;
    }
    review.footer();
    review.screen()?;
    review.control()?;
    Ok(review.finish())
}
