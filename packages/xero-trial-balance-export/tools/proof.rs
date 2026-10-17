use xero_trial_balance_export::{Failure, Result, offline, stdio};
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        stdio::out("usage: xero-tb-proof [--check]\n");
        return Ok(());
    }
    if !args.is_empty() && args != ["--check"] {
        return Err(Failure {
            code: 2,
            stdout: String::new(),
            stderr: "usage: xero-tb-proof [--check]\n".into(),
        });
    }
    let root = offline::component_root();
    let text = std::fs::read_to_string(root.join("samples/sample-output.csv"))
        .map_err(|_| Failure::message("the fabricated sample cannot be read"))?;
    let summary = offline::summary(&text)?;
    let assets = [
        ("assets/quick-proof.svg", offline::render_svg(&summary)),
        (
            "assets/quick-proof.md",
            offline::render_transcript(&summary),
        ),
    ];
    let check = args == ["--check"];
    let mut stale = false;
    for (name, expected) in assets {
        let path = root.join(name);
        if check {
            if !std::fs::read_to_string(path)
                .is_ok_and(|text| text.replace("\r\n", "\n") == expected)
            {
                stdio::out(&format!("stale: {name}\n"));
                stale = true;
            }
        } else {
            std::fs::write(path, expected)
                .map_err(|_| Failure::message("the fabricated proof asset cannot be written"))?;
            stdio::out(&format!("wrote {name}\n"));
        }
    }
    if stale {
        return Err(Failure {
            code: 1,
            stdout: String::new(),
            stderr: String::new(),
        });
    }
    if check {
        stdio::out("quick proof is current\n");
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        stdio::out(&error.stdout);
        stdio::err(&error.stderr);
        std::process::exit(error.code);
    }
}
