Attribute VB_Name = "modFormulaCompareTests"
Option Explicit

' Run manually in Excel after importing this module and modFormulaCompare.bas.
Public Sub TestCompareFormulaRanges()
    Dim beforeBook As Workbook, afterBook As Workbook
    Dim beforeSheet As Worksheet, afterSheet As Worksheet
    Dim findings As Collection, item As Variant, row As Long
    Dim failureNumber As Long, failureText As String
    On Error GoTo Failed
    Set beforeBook = Workbooks.Add(xlWBATWorksheet)
    Set afterBook = Workbooks.Add(xlWBATWorksheet)
    Set beforeSheet = beforeBook.Worksheets(1)
    Set afterSheet = afterBook.Worksheets(1)
    For row = 1 To 4
        beforeSheet.Cells(row, 2).Value2 = row
        afterSheet.Cells(row, 2).Value2 = row
        beforeSheet.Cells(row, 1).Formula2 = "=SUM(B1:B4)"
        afterSheet.Cells(row, 1).Formula2 = "=SUM(B1:B4)"
    Next row
    afterSheet.Range("A1").Value2 = 10
    afterSheet.Range("A2").Formula2 = "=SUM(B1:B3)+B4"
    beforeSheet.Range("A3").Value2 = 10
    afterSheet.Range("A4").Font.Bold = True
    beforeSheet.Calculate
    afterSheet.Calculate
    For row = 1 To 4
        Require beforeSheet.Cells(row, 1).Value2 = afterSheet.Cells(row, 1).Value2, _
                "The equal-value fixtures must agree."
    Next row
    Set findings = CompareFormulaRanges(beforeSheet.Range("A1:A4"), afterSheet.Range("A1:A4"))
    Require findings.Count = 3, "Formatting alone must not add a formula finding."
    item = findings(1)
    Require item(1) = "FORMULA_REMOVED", "A constant must expose formula removal."
    item = findings(2)
    Require item(1) = "FORMULA_CHANGED", "Changed formula text must be reported."
    item = findings(3)
    Require item(1) = "FORMULA_ADDED", "A new formula must be reported."

    afterSheet.Range("A4").Formula2 = "='[" & beforeBook.Name & "]" _
                                    & beforeSheet.Name & "'!$A$1"
    Set findings = CompareFormulaRanges(beforeSheet.Range("A1:A3"), afterSheet.Range("A1:A3"))
    Require findings.Count = 4, "A link outside the selected cells is workbook-level evidence."
    item = findings(4)
    Require item(0) = "WORKBOOK" And item(1) = "EXTERNAL_LINK_ADDED", _
            "A new workbook link must be reported."
    Require afterSheet.Range("A1").Value2 = 10, "The diagnostic must preserve constants."
    Require afterSheet.Range("A2").Formula2 = "=SUM(B1:B3)+B4", _
            "The diagnostic must preserve formulas."
    Require afterSheet.Range("A4").HasFormula, "The diagnostic must preserve links."
    Refuses beforeSheet.Range("A1"), afterSheet.Range("A1:A2")
    Refuses beforeSheet.Range("A1,A3"), afterSheet.Range("A1:A2")
    Refuses beforeSheet.Range("A1:A100001"), afterSheet.Range("A1:A100001")
    Refuses Nothing, afterSheet.Range("A1")
    afterBook.Close SaveChanges:=False
    beforeBook.Close SaveChanges:=False
    Debug.Print "TestCompareFormulaRanges: PASS"
    Exit Sub
Failed:
    failureNumber = Err.Number
    failureText = Err.Description
    On Error Resume Next
    If Not afterBook Is Nothing Then afterBook.Close SaveChanges:=False
    If Not beforeBook Is Nothing Then beforeBook.Close SaveChanges:=False
    On Error GoTo 0
    Err.Raise failureNumber, "TestCompareFormulaRanges", failureText
End Sub

Private Sub Require(ByVal condition As Boolean, ByVal message As String)
    If Not condition Then Err.Raise vbObjectError + 1, "TestCompareFormulaRanges", message
End Sub

Private Sub Refuses(ByVal beforeRange As Range, ByVal afterRange As Range)
    Dim findings As Collection, failureNumber As Long
    On Error Resume Next
    Set findings = CompareFormulaRanges(beforeRange, afterRange)
    failureNumber = Err.Number
    On Error GoTo 0
    Require failureNumber = 5, "Invalid ranges must be refused with error 5."
End Sub
