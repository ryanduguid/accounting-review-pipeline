// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Safety pins on the VBA modules `modReconCompare` and `modWorkpaperFormat`.

mod common;

use common::{Source, assert_crlf_only};

const RECON: &str = "vba/modReconCompare.bas";
const WORKPAPER: &str = "vba/modWorkpaperFormat.bas";

fn recon() -> Source {
    Source::adapter(RECON)
}

fn workpaper() -> Source {
    Source::adapter(WORKPAPER)
}

/// One Public Sub's text, so a guard is pinned to the sub it belongs to instead of
/// merely being counted somewhere in the module.
fn sub_body(name: &str) -> Source {
    workpaper()
        .starting_at(&format!("Public Sub {name}"))
        .until("End Sub")
}

#[test]
fn reconciliation_refuses_missing_ranges_and_negative_tolerance() {
    recon().has_all(&[
        "If rangeA Is Nothing Or rangeB Is Nothing Then",
        "If tolerance < 0 Then",
    ]);
}

#[test]
fn vba_requires_the_generated_sheet_marker_before_deleting() {
    let source = recon();
    source.has_all(&[
        "Private Const RESULT_TAG_NAME As String = \"__ReconCompareResultSheet\"",
        "If Not IsGeneratedResultSheet(wb, stale) Then",
        "If Not marker Is Nothing Then",
        "previousAlerts = Application.DisplayAlerts",
        "Application.DisplayAlerts = previousAlerts",
    ]);
    source.before(
        "If Not IsGeneratedResultSheet(wb, stale) Then",
        "stale.Delete",
    );
    source.lacks("wb.Sheets(\"Recon Result\").Delete");
    assert_crlf_only(RECON);
}

/// Deleting the generated sheet by hand used to brick the macro: the leftover marker is
/// a HIDDEN defined name, invisible in Name Manager, and the error told the user to
/// remove it. The no-sheet branch must clean up a dead marker itself and only stop when
/// the marker still resolves to a live (renamed, kept) sheet, with Excel-UI steps.
#[test]
fn dead_marker_is_cleaned_up_instead_of_raised_at_the_user() {
    let source = recon();
    let branch = source.between(
        "If stale Is Nothing Then",
        "If Not IsGeneratedResultSheet(wb, stale) Then",
    );
    branch.has_all(&[
        "Set keptSheet = MarkerTaggedSheet(wb, marker)",
        "If keptSheet Is Nothing Then",
        "marker.Delete",
        "Rename it back to",
    ]);
    branch.before("If keptSheet Is Nothing Then", "marker.Delete");
    source.lacks("remove or rename the marker");
}

/// Excel refuses to delete the last visible sheet, and with DisplayAlerts off it refuses
/// silently. The module must re-check the sheet after the delete and raise BEFORE
/// removing the marker, so the surviving sheet stays recognisable as generated on the
/// next run, and the raise must run with the DisplayAlerts-restoring handler armed.
#[test]
fn survived_delete_is_caught_before_the_marker_is_dropped() {
    let source = recon();
    let recheck = "Set survivor = wb.Sheets(RESULT_SHEET_NAME)";
    let raise = "only visible sheet";
    source.in_order(&[
        "stale.Delete",
        recheck,
        raise,
        "If Not marker Is Nothing Then marker.Delete",
    ]);
    source
        .between(recheck, raise)
        .has("On Error GoTo DeleteFailed");
}

/// The marker used to anchor ownership to the deletable cell $A$1: removing row 1 or
/// column A broke it to #REF! and the module disowned its own sheet. Identity must ride
/// on the sheet's CodeName, stored as a text constant, with the legacy range anchor
/// still recognised.
#[test]
fn marker_identity_survives_cell_edits_on_the_result_sheet() {
    let source = recon();
    source.has_all(&[
        "RefersTo:=\"=\"\"\" & ws.CodeName & \"\"\"\"",
        "Private Function StoredCodeName",
        "Private Function SheetWithCodeName",
    ]);
    let recognise = source
        .starting_at("Private Function IsGeneratedResultSheet")
        .until("End Function");
    recognise.has("MarkerTaggedSheet(wb, marker)");
    recognise.lacks("RefersToRange");
    let legacy = source
        .starting_at("Private Function MarkerTaggedSheet")
        .until("End Function");
    legacy.has("RefersToRange");
}

// modWorkpaperFormat's write-safety guards, which nothing pinned before they were added.
// This is not a claim about the sibling module: the checks above cover only
// modReconCompare's Nothing and negative-tolerance checks and its result-sheet marker
// machinery; several of that module's other guards are still unpinned and could be
// deleted with the suite green.

/// An entity name starting with "=" ("=Smith & Co Pty Ltd" is a real trading name shape
/// once someone pastes from a formula cell) is stored as a live formula unless the cells
/// are text-formatted FIRST, and the client name then renders as #NAME? on every page of
/// the pack.
#[test]
fn header_cells_are_forced_to_text_before_the_write() {
    workpaper().before(
        ".Range(\"A1:A4\").NumberFormat = \"@\"",
        ".Range(\"A1\").Value = entityName",
    );
}

/// A live cut or copy marquee turns Insert into a paste, so the clipboard block lands in
/// rows 1:5 instead of blank rows. The inserted block and FreezeBelowHeader's default
/// have to stay the same 5 rows, or the freeze lands inside the header.
#[test]
fn header_insert_clears_the_clipboard_and_matches_the_freeze_default() {
    let source = workpaper();
    source.has_all(&[
        "Application.CutCopyMode = False",
        "ws.Rows(\"1:5\").Insert Shift:=xlDown",
        "Optional ByVal headerRows As Long = 5",
    ]);
    source.before("Application.CutCopyMode = False", "ws.Rows(\"1:5\").Insert");
}

/// Every sub that takes a Worksheet refuses a protected one, checked inside each sub
/// rather than by counting the guard module-wide.
///
/// This deliberately does NOT claim every writing sub is guarded: the fourth public sub,
/// FormatAsAccounting, takes a Range and carries no worksheet-level guard, so a name
/// promising all 4 would be false. The module-wide count is a floor rather than an
/// equality for the same reason: pinning it at exactly 3 would fail the very change that
/// adds a guard to FormatAsAccounting.
#[test]
fn each_worksheet_taking_sub_refuses_a_protected_sheet() {
    let guard = "If ws.ProtectContents Then Err.Raise 5";
    for name in [
        "ApplyWorkpaperHeader",
        "AddReviewerLine",
        "FreezeBelowHeader",
    ] {
        sub_body(name).has(guard);
    }
    let count = workpaper().count(guard);
    assert!(
        count >= 3,
        "{WORKPAPER} carries the protected-sheet guard {count} times"
    );
    sub_body("FreezeBelowHeader").has_all(&[
        "If headerRows < 1 Then Err.Raise 5",
        "If ws.Visible <> xlSheetVisible Then Err.Raise 5",
    ]);
}

/// Find inherits the user's last Find-dialog settings when they are not passed. With
/// LookIn:=xlValues a formula-only footer row is invisible, and the reviewer sign-off is
/// written over live data.
///
/// SearchFormat is the same trap from the other end and has to be pinned with the other
/// 4: it is dialog-sticky too, so a user who last ran a Find restricted to a cell format
/// leaves that format in force, "*" matches nothing on the sheet, Find returns Nothing,
/// lastRow falls to 0, and the sign-off lands on A2, over the workpaper title.
#[test]
fn last_row_search_reads_formulas_and_pins_its_own_settings() {
    workpaper().has_all(&[
        "LookIn:=xlFormulas",
        "LookAt:=xlPart",
        "SearchOrder:=xlByRows",
        "SearchDirection:=xlPrevious",
        "SearchFormat:=False",
    ]);
}

/// Each module imports stand-alone, so the accounting number format lives as a private
/// copy in modWorkpaperFormat AND modReconCompare rather than as one shared constant.
/// Nothing tied the copies together: edit one (a different zero dash, a red-negative
/// variant) and workpapers and recon results render the same amounts differently, with
/// the suite green. Pinned as one exact declaration in both files, like the header-slice
/// pair, so a deliberate format change has to land in both modules and here. The count
/// keeps each module to the single constant, so a third inline copy cannot creep back in
/// beside it.
#[test]
fn accounting_number_format_is_identical_in_both_modules() {
    let format = "\"#,##0.00_);(#,##0.00);\"\"-\"\"??_)\"";
    let declaration = format!("Private Const ACCOUNTING_NUMBER_FORMAT As String = {format}");
    let (recon, workpaper) = (recon(), workpaper());
    for source in [&recon, &workpaper] {
        source.has(&declaration);
        assert_eq!(
            source.count(format),
            1,
            "copies of the format in {}",
            source.name
        );
    }
    // Binding the constant is not using it.
    recon.has(".NumberFormat = ACCOUNTING_NUMBER_FORMAT");
    workpaper.has("target.NumberFormat = ACCOUNTING_NUMBER_FORMAT");
}

/// CRLF means both halves. Stripping CRLF pairs and looking only for a bare LF passes a
/// CR-only file untouched (nothing is stripped and there is no LF to find), and the VBE
/// imports CR-only source as a single line, destroying the module.
#[test]
fn module_stays_crlf_for_the_vbe_import() {
    assert_crlf_only(WORKPAPER);
}
