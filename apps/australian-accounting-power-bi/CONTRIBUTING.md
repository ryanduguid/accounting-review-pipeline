# Contributing

Thank you for your interest in improving Australian Accounting Power BI.

## Core Principles

1. **Source as Text**: Everything lives in text formats (`.pbip`, `.tmdl`, `.pbir`, `.json`, `.py`). Power Query M is embedded in `expressions.tmdl`; monolithic binary `.pbix` files are not committed to git.
2. **Zero Client Data**: Real taxpayer, employee, bank, or ledger data must never enter this repository.
3. **Fixture Naming Conventions**:
   - Clean, standard test fixtures use neutral invented place names (e.g. *Varrock Ventures*, *Draynor Produce*, *Falador Freight*, *Ardougne Holdings*).
   - Adversarial edge-case fixtures use fictional antagonist company names (e.g. *Fiamma Nera Salvage*).
4. **Verification First**: Pull requests must pass automated tests in `tests/`, including double-entry GL balance verification, TMDL schema validation, and Power Query syntax checks.

## Development Workflow

1. Clone the repository and inspect or edit TMDL / M files using your preferred code editor (VS Code with TMDL extension, Tabular Editor, or Power BI Desktop with developer mode enabled).
2. Run automated test suites:
   ```bash
   python -B -m unittest discover -s tests -v
   npx --yes @microsoft/powerbi-report-authoring-cli@0.1.4 validate australian-accounting-power-bi.Report
   ```
3. Submit a pull request adhering to Conventional Commits format (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`).
