Attribute VB_Name = "modReconCompare"
Option Explicit

Private Const RESULT_SHEET_NAME As String = "Recon Result"
Private Const RESULT_TAG_NAME As String = "__ReconCompareResultSheet"

' Kept byte-identical to ACCOUNTING_NUMBER_FORMAT in modWorkpaperFormat.bas -
' each module imports stand-alone, so each carries its own copy of the
' accounting format; tests/test_static_guards.py pins the two together.
Private Const ACCOUNTING_NUMBER_FORMAT As String = "#,##0.00_);(#,##0.00);""-""??_)"

' modReconCompare
' Keyed two-way reconciliation between two ranges - the "why doesn't the
' subledger agree to the GL" workhorse. Late-bound Scripting.Dictionary,
' so no references need adding. Windows Excel only - Mac Excel has no
' Scripting.Dictionary and no reference can supply it.
' Import via VBE: File > Import File...

' Compares two two-column ranges (key, amount). Writes a "Recon Result"
' sheet listing: keys only in A or only in B whose summed amount exceeds
' tolerance, and keys in both where the amounts differ by more than
' tolerance. Duplicate keys within a side are summed before comparing
' (subledger detail vs GL balance pattern).
'
' Keys compare as trimmed text, case-insensitive. A key stored as TEXT
' "001234" on one side and as the NUMBER 1234 on the other normalises to
' "001234" vs "1234" - two different keys, reported as two one-sided
' exceptions. Format both key columns the same way before running.
'
' Invisible characters ride in with pasted data: non-breaking spaces, tabs
' and line breaks normalise to plain spaces before trimming, zero-width
' spaces drop out. Trim$ alone leaves them, and a trailing non-breaking
' space reports the same key unmatched on both sides with nothing visible
' to explain it.
'
' Example:
'   CompareKeyedRanges Sheet1.Range("A2:B500"), Sheet2.Range("A2:B300"), 0.01
Public Sub CompareKeyedRanges( _
    ByVal rangeA As Range, _
    ByVal rangeB As Range, _
    Optional ByVal tolerance As Double = 0.005)

#If Mac Then
    ' Scripting.Dictionary lives in the Windows-only scripting runtime - name
    ' the platform instead of failing with a bare 429 on the first CreateObject.
    Err.Raise 5, , "modReconCompare needs Scripting.Dictionary - Windows Excel only."
#End If

    If rangeA Is Nothing Or rangeB Is Nothing Then
        Err.Raise 5, , "Both source ranges are required."
    End If
    If tolerance < 0 Then
        Err.Raise 5, , "Tolerance must be zero or greater."
    End If

    Dim skippedRows As Long
    skippedRows = 0

    Dim dictA As Object, dictB As Object
    Set dictA = SumByKey(rangeA, skippedRows)
    Set dictB = SumByKey(rangeB, skippedRows)

    ' Refuse to run when a source range lives on the result sheet - the
    ' delete below would destroy caller data.
    If StrComp(rangeA.Worksheet.Name, RESULT_SHEET_NAME, vbTextCompare) = 0 _
        Or StrComp(rangeB.Worksheet.Name, RESULT_SHEET_NAME, vbTextCompare) = 0 Then
        Err.Raise 5, , "Source range is on the 'Recon Result' sheet - move the data or rename the sheet."
    End If

    Dim wb As Workbook
    Set wb = rangeA.Worksheet.Parent
    If wb.ProtectStructure Then
        Err.Raise 5, , "Workbook structure is protected - unprotect it before running the recon."
    End If

    ' Only a sheet marked by this module is replaceable. An unrelated sheet
    ' called "Recon Result" can contain real user work and must never be
    ' deleted just because it shares the result name.
    DeletePreviousGeneratedResult wb

    Dim ws As Worksheet
    Set ws = wb.Worksheets.Add
    ws.Name = RESULT_SHEET_NAME
    MarkGeneratedResultSheet wb, ws

    ws.Range("A1:D1").Value = Array("Key", "Side A", "Side B", "Difference")
    ws.Range("A1:D1").Font.Bold = True

    Dim outRow As Long
    outRow = 2

    Dim k As Variant, amtA As Double, amtB As Double, diff As Double
    ' Keys in A (matched and A-only)
    For Each k In dictA.Keys
        amtA = dictA(k)
        amtB = 0#
        If dictB.Exists(k) Then amtB = dictB(k)
        diff = amtA - amtB
        If Abs(diff) > tolerance Then
            ' Text format BEFORE the write - .Value into a General cell
            ' re-parses "001234" to 1234 and "3-10" to a date
            ws.Cells(outRow, 1).NumberFormat = "@"
            ws.Cells(outRow, 1).Value = k
            ws.Cells(outRow, 2).Value = amtA
            If dictB.Exists(k) Then ws.Cells(outRow, 3).Value = amtB
            ws.Cells(outRow, 4).Value = diff
            outRow = outRow + 1
        End If
    Next k
    ' Keys only in B
    For Each k In dictB.Keys
        If Not dictA.Exists(k) Then
            amtB = dictB(k)
            If Abs(amtB) > tolerance Then
                ws.Cells(outRow, 1).NumberFormat = "@"
                ws.Cells(outRow, 1).Value = k
                ws.Cells(outRow, 3).Value = amtB
                ws.Cells(outRow, 4).Value = -amtB
                outRow = outRow + 1
            End If
        End If
    Next k

    ' A clean recon leaves outRow = 2 - the reversed corner pair would then
    ' normalise to the B1:D2 bounding box and format the header row.
    If outRow > 2 Then
        ws.Range(ws.Cells(2, 2), ws.Cells(outRow - 1, 4)).NumberFormat = ACCOUNTING_NUMBER_FORMAT
    End If
    ws.Columns("A:D").AutoFit

    ws.Cells(outRow + 1, 1).Value = "Items: " & (outRow - 2) & _
        "   Skipped rows (errors/blanks): " & skippedRows & _
        "   Tolerance: " & tolerance & _
        "   Run: " & Format$(Now, "d mmm yyyy hh:mm")
End Sub

Private Sub DeletePreviousGeneratedResult(ByVal wb As Workbook)
    Dim stale As Object
    Dim marker As Name
    Dim keptSheet As Object
    Dim survivor As Object
    Dim previousAlerts As Boolean

    On Error Resume Next
    Set stale = wb.Sheets(RESULT_SHEET_NAME)
    On Error GoTo 0
    If stale Is Nothing Then
        On Error Resume Next
        Set marker = wb.Names(RESULT_TAG_NAME)
        On Error GoTo 0
        If marker Is Nothing Then Exit Sub

        ' The marker is a HIDDEN name: it never shows in Name Manager, so
        ' an error demanding its removal cannot be acted on from the Excel
        ' UI. When the sheet it tagged is gone (deleted by hand), the
        ' marker is dead - clean it up and carry on. Only a marker still
        ' attached to a live sheet (the generated sheet was renamed and
        ' kept) is worth stopping for, and that stop names UI-only steps.
        Set keptSheet = MarkerTaggedSheet(wb, marker)
        If keptSheet Is Nothing Then
            marker.Delete
            Exit Sub
        End If
        Err.Raise 5, , "A previous recon result sheet was renamed to '" & keptSheet.Name & _
            "' and is still marked as generated. Rename it back to '" & RESULT_SHEET_NAME & _
            "' to let the recon replace it, or to keep it: make a copy of the sheet " & _
            "(right-click its tab > Move or Copy), then delete the original."
    End If

    If Not IsGeneratedResultSheet(wb, stale) Then
        Err.Raise 5, , "A sheet named 'Recon Result' already exists but was not generated by modReconCompare. It was left untouched; rename or remove it before running the recon."
    End If

    previousAlerts = Application.DisplayAlerts
    On Error GoTo DeleteFailed
    Application.DisplayAlerts = False
    stale.Delete

    ' Excel refuses to delete the last visible sheet, and with
    ' DisplayAlerts off it refuses SILENTLY: .Delete returns with the
    ' sheet still in place. Re-check and stop BEFORE touching the marker
    ' so the surviving sheet stays marked and is still recognised as
    ' generated on the next run. The raise lands in DeleteFailed, which
    ' restores DisplayAlerts.
    On Error Resume Next
    Set survivor = wb.Sheets(RESULT_SHEET_NAME)
    On Error GoTo DeleteFailed
    If Not survivor Is Nothing Then
        Err.Raise 5, , "'" & RESULT_SHEET_NAME & "' is the only visible sheet, so Excel cannot delete it. Add or unhide another sheet, then run the recon again."
    End If

    Application.DisplayAlerts = previousAlerts

    ' Deleting a worksheet normally leaves the workbook-level marker as a
    ' #REF! name, but Excel may remove it itself in some workbook formats.
    ' Either outcome is safe; only a remaining marker needs explicit cleanup.
    On Error Resume Next
    Set marker = wb.Names(RESULT_TAG_NAME)
    On Error GoTo DeleteFailed
    If Not marker Is Nothing Then marker.Delete
    Exit Sub

DeleteFailed:
    Application.DisplayAlerts = previousAlerts
    Err.Raise Err.Number, , "Cannot replace the generated 'Recon Result' sheet: " & Err.Description
End Sub

Private Function IsGeneratedResultSheet(ByVal wb As Workbook, ByVal candidate As Object) As Boolean
    Dim marker As Name
    Dim tagged As Object

    If TypeName(candidate) <> "Worksheet" Then Exit Function

    On Error Resume Next
    Set marker = wb.Names(RESULT_TAG_NAME)
    On Error GoTo 0
    If marker Is Nothing Then Exit Function

    Set tagged = MarkerTaggedSheet(wb, marker)
    If tagged Is Nothing Then Exit Function
    IsGeneratedResultSheet = (tagged Is candidate)
End Function

Private Sub MarkGeneratedResultSheet(ByVal wb As Workbook, ByVal ws As Worksheet)
    Dim existing As Name

    On Error Resume Next
    Set existing = wb.Names(RESULT_TAG_NAME)
    On Error GoTo 0
    If Not existing Is Nothing Then
        Err.Raise 5, , "Cannot mark the generated 'Recon Result' sheet because the reserved result marker already exists. Remove the stale marker before running the recon."
    End If

    ' Anchor the marker to the sheet's CodeName, stored as a text constant.
    ' A cell anchor like '<sheet>'!$A$1 breaks to #REF! as soon as the user
    ' deletes row 1 or column A of the result sheet, and the module would
    ' then disown its own output. The CodeName survives every cell edit. A
    ' password-locked VBA project leaves a new sheet's CodeName empty -
    ' fall back to the legacy cell anchor there, which MarkerTaggedSheet
    ' still understands.
    If Len(ws.CodeName) > 0 Then
        wb.Names.Add Name:=RESULT_TAG_NAME, _
            RefersTo:="=""" & ws.CodeName & """", _
            Visible:=False
    Else
        wb.Names.Add Name:=RESULT_TAG_NAME, _
            RefersTo:="='" & Replace$(ws.Name, "'", "''") & "'!$A$1", _
            Visible:=False
    End If
End Sub

' Resolves the marker back to the live worksheet it tags, or Nothing when
' that sheet no longer exists. Understands both marker formats: the
' CodeName text constant written by MarkGeneratedResultSheet, and the
' legacy '<sheet>'!$A$1 range anchor written by earlier module versions.
Private Function MarkerTaggedSheet(ByVal wb As Workbook, ByVal marker As Name) As Object
    Dim storedCode As String
    Dim taggedRange As Range

    storedCode = StoredCodeName(marker)
    If Len(storedCode) > 0 Then
        Set MarkerTaggedSheet = SheetWithCodeName(wb, storedCode)
        Exit Function
    End If

    ' Legacy range anchor: deleting the tagged sheet leaves it as #REF!
    ' and the probe errors; renaming the tagged sheet keeps it resolving
    ' to a live range.
    On Error Resume Next
    Set taggedRange = marker.RefersToRange
    On Error GoTo 0
    If Not taggedRange Is Nothing Then Set MarkerTaggedSheet = taggedRange.Worksheet
End Function

' Extracts the CodeName from a text-constant marker (RefersTo ="Sheet3").
' Returns "" for any other RefersTo shape (legacy range anchor, #REF!).
Private Function StoredCodeName(ByVal marker As Name) As String
    Dim refersText As String
    refersText = marker.RefersTo
    If Len(refersText) < 4 Then Exit Function
    If Left$(refersText, 2) <> "=""" Then Exit Function
    If Right$(refersText, 1) <> """" Then Exit Function
    StoredCodeName = Replace$(Mid$(refersText, 3, Len(refersText) - 3), """""", """")
End Function

Private Function SheetWithCodeName(ByVal wb As Workbook, ByVal targetCodeName As String) As Object
    Dim sh As Worksheet
    For Each sh In wb.Worksheets
        ' CodeNames are VBA identifiers, unique case-insensitively.
        If StrComp(sh.CodeName, targetCodeName, vbTextCompare) = 0 Then
            Set SheetWithCodeName = sh
            Exit Function
        End If
    Next sh
End Function

' Sums a two-column (key, amount) range into a dictionary, keyed on the
' trimmed text of column 1. Rows with error values (#N/A, #REF!...), blank
' keys, or blank/non-numeric amounts are skipped and counted, not crashed on.
Private Function SumByKey(ByVal source As Range, ByRef skippedRows As Long) As Object
    Dim dict As Object
    Set dict = CreateObject("Scripting.Dictionary")
    dict.CompareMode = vbTextCompare

    ' Shape checks - Cells(r, 2) on a one-column range would read the
    ' worksheet column beside it, and a Ctrl-selected union silently
    ' truncates to its first area.
    If source.Areas.Count > 1 Then
        Err.Raise 5, , "Pass a single contiguous range - a Ctrl-selected union would be truncated to its first area."
    End If
    If source.Columns.Count < 2 Then
        Err.Raise 5, , "Range needs at least two columns (key, amount)."
    End If

    ' Bound the loop to the used range - a whole-column selection (A:B) is
    ' 1,048,576 rows and two COM reads per row, which freezes Excel for
    ' minutes. Rows only; column geometry stays exactly as passed.
    Dim used As Range, lastR As Long
    Set used = Intersect(source, source.Worksheet.UsedRange)
    If used Is Nothing Then
        Set SumByKey = dict
        Exit Function
    End If
    lastR = used.Row + used.Rows.Count - source.Row

    Dim r As Long, k As String, keyVal As Variant, v As Variant
    For r = 1 To lastR
        keyVal = source.Cells(r, 1).Value
        v = source.Cells(r, 2).Value
        If IsError(keyVal) Or IsError(v) Then
            skippedRows = skippedRows + 1
        Else
            ' Trim$ only sees plain spaces - a non-breaking space, tab or
            ' line break pasted in with a key leaves it looking identical to
            ' a clean one and matching nothing.
            k = CStr(keyVal)
            k = Replace$(k, ChrW$(160), " ")
            k = Replace$(k, vbTab, " ")
            k = Replace$(k, vbCrLf, " ")
            k = Replace$(k, vbCr, " ")
            k = Replace$(k, vbLf, " ")
            k = Replace$(k, ChrW$(8203), "")
            k = Trim$(k)
            ' Not IsEmpty guards the VBA trap IsNumeric(Empty) = True - a
            ' blank amount must count as skipped, not sum as a silent zero.
            ' VarType guards the sibling trap IsNumeric(True) = True with
            ' CDbl(True) = -1 - a stray TRUE must skip, not sum as -1.00
            If Len(k) > 0 And Not IsEmpty(v) And VarType(v) <> vbBoolean And IsNumeric(v) Then
                If dict.Exists(k) Then
                    dict(k) = dict(k) + CDbl(v)
                Else
                    dict.Add k, CDbl(v)
                End If
            ElseIf Len(k) > 0 Or Not IsEmpty(v) Then
                ' counts blank/bad-amount rows AND blank-key rows with data;
                ' fully empty rows (oversized selections) stay uncounted
                skippedRows = skippedRows + 1
            End If
        End If
    Next r

    Set SumByKey = dict
End Function
