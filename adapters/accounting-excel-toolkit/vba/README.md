# VBA modules

Source lives here as importable text, never only inside a binary workbook. GitHub can render, diff and review text; it can't see inside an `.xlsm`.

The VBE exports three text formats and reads all three back: `.bas` for a standard module, `.cls` for a class module, `.frm` for a form. Today this folder holds two `.bas` modules, but the tooling covers all three so a class or form dropped in here is checked from the moment it lands. The `.frx` companion a form export also writes is binary by design and is left alone.

## Import

1. Open the VBA editor (`Alt+F11`)
2. File > Import File... > pick the module
3. Save the workbook as `.xlsm`

The VBE reads that source as system ANSI, not UTF-8, and offers no encoding
choice on import. Keep it pure ASCII with CRLF line endings so the read is
lossless: an em dash or a smart quote in a comment arrives as mojibake.
`.gitattributes` pins `*.bas`, `*.cls` and `*.frm` to CRLF, and `python
tools/check_vba_encoding.py` checks the encoding, the line endings and the
absence of a BOM across the whole folder, subdirectories included.

Both modules are self-contained: no library references to add (`modReconCompare` late-binds `Scripting.Dictionary`). That binding also pins `modReconCompare` to Windows Excel. Mac Excel has no `Scripting.Dictionary` and no reference can supply it. `modWorkpaperFormat` runs on both.

## Modules

| Module | What it does |
|---|---|
| `modWorkpaperFormat` | Standard workpaper header block, reviewer sign-off line, accounting number format, freeze panes |
| `modReconCompare` | Keyed two-way reconciliation between two (key, amount) ranges with tolerance; subledger vs GL pattern. It replaces only a prior result sheet that it marked itself; a user-created sheet named `Recon Result` is left untouched and the macro stops with instructions |

## Contributing your own

Export a module as text before committing: right-click the module in the VBE → Export File… Keep `Option Explicit` on and note any required references in the module header comment. Run `python tools/check_vba_encoding.py` before you commit; the VBE exports UTF-8 when a comment contains a non-ASCII character, and that file will not import back cleanly.
