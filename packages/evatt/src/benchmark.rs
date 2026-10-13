//! Feature-gated measurements, reusing the original four timing families.
use crate::{
    patterns::{self, Result},
    pyjson::Json,
    redact,
};
use std::time::Instant;

fn text(request: &Json, key: &str) -> String {
    request.at(key).as_pystr().unwrap().to_utf8().unwrap()
}

fn run(request: &Json, source: &str) -> Result<()> {
    match text(request, "family").as_str() {
        "label" => {
            assert!(!patterns::has(&text(request, "pattern"), source)?);
        }
        "name" => {
            patterns::person_names(source)?;
        }
        "value" => {
            assert!(
                !patterns::value_pattern(&text(request, "value"))?
                    .is_match(source)
                    .map_err(patterns::runtime)?
            );
        }
        "number" => {
            redact::residual(source)?;
        }
        _ => return Err("unknown benchmark family".into()),
    }
    Ok(())
}

fn source(request: &Json, size: usize) -> String {
    if request.get("markup").is_some_and(|m| matches!(m, Json::Bool(true))) {
        let padding = text(request, "padding");
        return format!(
            "{}{}{}:{}x",
            text(request, "prefix"),
            format!("*_{padding}`").repeat(size),
            " ".repeat(size),
            format!("`{padding}_*").repeat(size)
        );
    }
    format!(
        "{}{}{}",
        text(request, "prefix"),
        text(request, "unit").repeat(size),
        text(request, "suffix")
    )
}

pub fn measure(request: &Json) -> Result<Json> {
    use num_traits::ToPrimitive;
    let size = request.at("size").as_plain_int().unwrap().to_usize().unwrap();
    let source = source(request, size);
    run(request, &source)?;
    let mut best = f64::INFINITY;
    for _ in 0..3 {
        let started = Instant::now();
        run(request, &source)?;
        best = best.min(started.elapsed().as_secs_f64());
    }
    let (candidates, retries, examined) = match text(request, "family").as_str() {
        "name" => {
            let attempts = patterns::name_attempts(&source)?;
            (attempts.len(), attempts.iter().filter(|a| a.2).count(), 0)
        }
        "number" => {
            let (_, stats) = redact::numbers_counted(&source)?;
            (stats.candidate_starts, 0, stats.characters_examined)
        }
        _ => (0, 0, 0),
    };
    Ok(Json::object([
        ("seconds", Json::Float(best)),
        ("code_points", Json::int(source.chars().count() as i64)),
        ("candidate_starts", Json::int(candidates as i64)),
        ("retries", Json::int(retries as i64)),
        ("characters_examined", Json::int(examined as i64)),
    ]))
}
