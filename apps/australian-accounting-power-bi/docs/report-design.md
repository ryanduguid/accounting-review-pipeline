# Report design

The report follows a financial review sequence: identify the selected period and entities, read the main results, compare them with prior results or budget, then inspect the supporting records. All values come from the fabricated local model.

## Reference dashboards

The design uses specific patterns from published examples. These references are useful benchmarks, not an objective ranking of the best dashboards.

| Reference | Pattern used here |
| --- | --- |
| Microsoft's [Corporate Spend sample](https://learn.microsoft.com/en-us/power-bi/create-reports/sample-corporate-spend) | Clearly labelled financial results, plan variance beside actuals, and tables supported by trends |
| Microsoft's [refreshed sample reports](https://learn.microsoft.com/en-us/power-bi/create-reports/sample-datasets), including Regional Sales | Native page navigation, consolidated card visuals, consistent spacing and restrained surfaces |
| Zebra BI's [financial dashboard examples](https://zebrabi.com/power-bi-financial-dashboards/) | Hierarchical financial statements, supporting detail and comparisons visible beside the current result |
| Microsoft's [accessible report guidance](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-accessibility-creating-reports) | Descriptive alt text, intentional tab order, contrast, and line markers that distinguish series without colour alone |

The report uses built-in Power BI visuals. The finance examples informed the structure; no vendor visual or new runtime dependency was added. The existing fact and dimension boundaries remain consistent with Microsoft's [star schema guidance](https://learn.microsoft.com/en-us/power-bi/guidance/star-schema).

## Page decisions

- **Financial performance:** one card visual contains four results. The P&L pairs actual, prior year and change. Revenue has its own budget comparison and monthly trend because the source does not supply a complete profit budget. Reporting basis is single-select on this page. The budget comparison and monthly revenue titles state when budget is unavailable; the comparison title directs the reviewer to the gross basis.
- **Group balances:** recorded balance-sheet accounts compare gross, elimination and net amounts. Equity excludes unclosed earnings in these fixtures, so the title states that limit. The journal detail supplies the audit trail. Consolidation remains restricted to the complete wholly owned AUD group.
- **Industry comparison:** a table exposes actual ratios and fabricated comparison values. Explicit numeric column widths separate adjacent percentage totals. Labelled horizontal bars replace the previous single-point scatter chart. Selecting multiple entities does not invent a combined industry benchmark.
- **Payroll receipts:** one card visual contains five results above the detail table. Plain receipt labels replace source codes. The note changes when a selection has no records, needs calendar review or uses the quarterly sample. Charge and interest totals remain unavailable when calendar review is required.
- **Close review:** the findings list shows account, status, grouped change and evidence state. Selecting one finding populates its full action, question, reason, evidence request and evidence state beside the list. The unselected state prompts a selection. Controls not run use readable labels.
- **Supporting journal rows:** the selected finding's amounts use the same grouping and parentheses for negative values. A separate audit details table exposes the complete run identifier. The journal table retains the full supporting records through native scrolling.

Each page has a context strip with entities, dates, currency and fabricated-data status. A discontinuous date selection is labelled as selected dates within a range. An empty date selection explains how to recover. Navigation retains each page's selections.

## Visual system

The registered `AccountingReview.json` theme centralises the palette and typography. Segoe UI retains the native Desktop appearance. Page titles use 22 pt, section titles and table text 12 pt, filter labels 11 pt, navigation 10 pt, and the main results 28 pt. All six pages use 20 px outer margins, 12 px gutters and a 4 px coordinate grid. Header navigation shares one position and size; filters share a 60 px height and align by column. The review pages reserve an extra line for their instructions. Paired financial tables and trend charts share their top and bottom edges. Numeric result cards use 28 pt values; the industry text result uses 14 pt so longer states can wrap. Every result card shares the same label placement, fill and padding. Tables use the same header fill, alternating rows and 2 px row padding. Review table headings use readable display labels and explicit column widths; native scrolling retains longer records and supporting context. Native controls retain their focus and interaction behaviour.

| Role | Colour |
| --- | --- |
| Main text | `#23384A` |
| Supporting text | `#526170` |
| Actual values and selected navigation | `#21665B` |
| Comparison values | `#315D8A` |
| Page | `#F3F5F6` |
| Table headers | `#E7EDEB` |
| Visual surfaces | `#FFFFFF` |

The checked text and series colours have contrast ratios from 5.36:1 to 12.10:1 against the three surface colours. Trend lines also use different marker shapes and dash styles. Bar values have labels and a supporting table. These checks do not establish full accessibility conformance.

## Verification and limits

The project has 6 pages, 59 visuals, 14 tables and 63 measures. Structural checks cover bindings, unique field aliases, visual bounds and overlap, the shared coordinate grid, navigation geometry, filter heights, card anatomy, table styles, non-empty alt text, unique tab positions and the compatibility-level floor. The tested Desktop build visits higher `tabOrder` values first, so the values descend along the visual reading order. The regression checks that order against page geometry. Native validation is recorded in [native verification](native-verification.md).

Review amount display columns group the original decimal strings without converting them to numbers or rounding. The raw values remain available for projection verification and audit context. Malformed display inputs fail refresh through the existing table-level error pattern. These display columns do not participate in financial calculations.

Run `powershell -NoProfile -File tools/measure_report_queries.ps1 -Server localhost:<port>` against a disposable, refreshed sample model to measure five representative queries. It records the first observed request, then the median and nearest-rank 95th percentile of 10 further requests. Timings include local engine round trips and row reads. They exclude visual rendering, network service latency and production concurrency. The first request is not guaranteed to use a cold cache.

All 59 visuals have separate portrait metadata across the six pages, with a 324-point canvas width, 8-point gaps, two-column navigation and full-width result cards. Phone table rows have extra padding, and dense tables retain horizontal scrolling. Selected-finding text wraps within the canvas. Native Desktop preview checks cover all page headers, populated results and the finding, journal evidence and Back flow. This preview does not establish physical-device or mobile-app acceptance.

Keyboard Enter and Ctrl+Enter operate the finding, evidence and Back actions after a known control is focused. NVDA's processed speech queue includes the finding, journal table and transaction cell. Full Tab traversal stops in Desktop text-editing controls, so complete keyboard and screen-reader acceptance remains unverified. Synthesiser queue text does not verify acoustic output or human comprehension.

A fabricated ledger experiment measured 1,718, 17,180 and 171,800 rows. Full refresh took 3.478, 6.288 and 50.942 seconds respectively. At the largest size, five warm query medians ranged from 3.65 to 12.91 ms. Complete balanced journals were repeated with unique identifiers; dimensions and other fixture tables were unchanged. These measurements exclude rendering, cold-cache guarantees, concurrent users and realistic production cardinality.

A subsequent alternating comparison at 171,800 fabricated rows measured three full refreshes per variant in the same Desktop instance. The original median was 46.022 seconds; buffering the existing entity and account key lists once per ledger evaluation reduced the median to 13.774 seconds (70.07% shorter). Original runs ranged from 43.362 to 52.593 seconds and buffered runs from 13.143 to 15.105 seconds. The comparison used repeated balanced journals with unique identifiers and unchanged dimensions. Independent row and money totals passed after every run. List.Contains comparisons, validation predicates and financial arithmetic are preserved. This result does not establish rendering speed, realistic production cardinality, Service latency or concurrency.

Power BI Service deployment, row-level security for real users, gateway refresh, production reconciliation and performance at realistic data volumes need a separate deployment brief. The model's accounting and source limitations remain in [model controls](model-controls.md).
