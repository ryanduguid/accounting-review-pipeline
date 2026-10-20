Attribute VB_Name = "modFormulaCompare"
Option Explicit

' Read-only diagnostic for aligned cells in two workbook revisions.
' Returns arrays: location, finding, before formula, after formula/link.
' Reads Formula2 text and workbook link metadata. Does not recalculate,
' open links, alter cells, write a report or approve accounting treatment.
Public Function CompareFormulaRanges( _
    ByVal beforeRange As Range, _
    ByVal afterRange As Range) As Collection

    If beforeRange Is Nothing Or afterRange Is Nothing Then
        Err.Raise 5, , "Both source ranges are required."
    End If
    If beforeRange.Areas.Count <> 1 Or afterRange.Areas.Count <> 1 Then
        Err.Raise 5, , "Select one contiguous range on each side."
    End If
    If beforeRange.Rows.Count <> afterRange.Rows.Count _
        Or beforeRange.Columns.Count <> afterRange.Columns.Count Then
        Err.Raise 5, , "Source ranges must have the same row and column counts."
    End If
    If beforeRange.CountLarge > 100000 Then
        Err.Raise 5, , "Compare at most 100000 aligned cells per call."
    End If

    Dim findings As New Collection
    Dim row As Long, column As Long
    Dim beforeCell As Range, afterCell As Range
    Dim beforeFormula As String, afterFormula As String, finding As String
    For row = 1 To beforeRange.Rows.Count
        For column = 1 To beforeRange.Columns.Count
            Set beforeCell = beforeRange.Cells(row, column)
            Set afterCell = afterRange.Cells(row, column)
            beforeFormula = ""
            afterFormula = ""
            finding = ""
            If beforeCell.HasFormula Then beforeFormula = CStr(beforeCell.Formula2)
            If afterCell.HasFormula Then afterFormula = CStr(afterCell.Formula2)
            If beforeCell.HasFormula And Not afterCell.HasFormula Then
                finding = "FORMULA_REMOVED"
            ElseIf Not beforeCell.HasFormula And afterCell.HasFormula Then
                finding = "FORMULA_ADDED"
            ElseIf StrComp(beforeFormula, afterFormula, vbBinaryCompare) <> 0 Then
                finding = "FORMULA_CHANGED"
            End If
            If Len(finding) > 0 Then
                findings.Add Array(afterCell.Address(False, False, xlA1, True), _
                                   finding, beforeFormula, afterFormula)
            End If
        Next column
    Next row

    ' Links are workbook-level observations, not claims about a selected cell.
    Dim oldLinks As Variant, newLinks As Variant, link As Variant, oldLink As Variant
    Dim existed As Boolean
    oldLinks = beforeRange.Worksheet.Parent.LinkSources(xlExcelLinks)
    newLinks = afterRange.Worksheet.Parent.LinkSources(xlExcelLinks)
    If IsArray(newLinks) Then
        For Each link In newLinks
            existed = False
            If IsArray(oldLinks) Then
                For Each oldLink In oldLinks
                    If StrComp(CStr(link), CStr(oldLink), vbTextCompare) = 0 Then
                        existed = True
                        Exit For
                    End If
                Next oldLink
            End If
            If Not existed Then
                findings.Add Array("WORKBOOK", "EXTERNAL_LINK_ADDED", "", CStr(link))
            End If
        Next link
    End If

    Set CompareFormulaRanges = findings
End Function
