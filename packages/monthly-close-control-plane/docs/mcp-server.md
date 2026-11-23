# Assistant access over MCP

`close-control mcp DIRECTORY [DIRECTORY ...]` serves the commands that only
read local files to an assistant, over standard input and output with the Model
Context Protocol. It writes no file, opens no network connection and approves
nothing. It runs until the assistant closes its input, then exits `0`; it exits
`1` when a directory does not exist or is not a directory.

## Tools

| Tool | Runs | Arguments |
| --- | --- | --- |
| `view` | `close-control view` | `pack_dir` |
| `queue` | `close-control queue` | `pack_dirs`, optional `period`, `status`, `reviewer`, `summary` |
| `compare` | `close-control compare` | `previous_pack`, `current_pack`, `previous_tb`, `current_tb`, optional `responses` |
| `classify` | `close-control classify` | `original_transactions`, `current_transactions`, `coding_evidence`, `currency` |
| `schedule` | `close-control schedule` | `kind`, `input` |
| `jet` | [`close-control jet`](journal-entry-tests.md) | `transactions`, `currency`, `period_end` |

Each tool returns the text its command prints: the review sheet, the queue or
the JSON result. A tool that cannot run, such as one given a file that does not
exist, returns its error text with `isError` set. A `REVIEW` result is not an
error. `review`, `workbench`, `reconcile`, `drivers` and `demo` write files, so
they are not tools: run them yourself, then let the assistant read the result.

## Which files a tool may read

Every path a tool is given must be absolute and must name an existing file or
directory inside one of the server's directories once links are resolved. The
path is resolved one name at a time, and a link is followed only where it sits
inside one of those directories, so a link elsewhere is never followed. When a
server directory was given through a link, write paths in it in their resolved
form. A
file must be a regular file, not a pipe or device. In a pack directory, each
pack file present must be a regular file itself, not a link, so reading it by
name stays inside the directory. Network and device paths (`\\server\share`,
`//server/share`) are refused before anything resolves them, both as tool
arguments and as the server's directories. Any other path is refused before the
command reads it.

The checks run just before the command opens the files, so someone who can
change the allowed folders at that moment, or who places a hard link to an
outside file in them, can still have it read. Give the server only the folders
the assistant needs, keep client folders outside any checkout, and do not
share them with people or programs you would not let read the files directly.

## Protocol

Messages are JSON-RPC 2.0, one per line. The server answers `initialize` for
protocol versions 2025-06-18 and 2025-11-25, and `server/discover` and
per-request `_meta` versions for 2026-07-28. It also answers `ping`,
`tools/list` and `tools/call`; other requests get `Method not found`, and
notifications get no reply.

## Claude Desktop and other clients

A client that starts local servers takes the executable and the directories:

```json
{
  "mcpServers": {
    "close-control": {
      "command": "/path/to/close-control",
      "args": ["mcp", "/path/to/client-files"]
    }
  }
}
```

[`mcpb/manifest.json`](../mcpb/manifest.json) describes the same server as an
MCP Bundle (`.mcpb`), which asks for the folders when it is installed. A bundle
holds the manifest and the release executable at `server/close-control`
(`server/close-control.exe` on Windows) and is packed with
`npx @anthropic-ai/mcpb pack`. Bundles are not yet attached to releases.
