# Assistant access over MCP

`close-control mcp DIRECTORY` serves the commands that only
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

The server opens `DIRECTORY` once when it starts and reads every file through
that open directory, never by a full path. A path a tool is given is relative
to it, written as names separated by `/`, such as `acme/2026-10/pack`. A path
that is absolute or empty, or has an empty name, `.`, `..`, `\` or `:` in it,
is refused, so no spelling can name a drive, a share, a device or a parent
folder. Each name is opened inside the folder opened before it, and a link,
junction or other reparse point at any step is refused rather than followed.
A file must be a regular file, not a pipe or device; this holds for the files
a command reads inside a pack directory too.

A link someone creates in the folder whilst a command runs can be followed, but
only to a place inside the folder, never outside it. A hard link to an outside
file placed in the folder can still have that file read. Give the server only
the folder the assistant needs, keep client folders outside any checkout, and
do not share it with people or programs you would not let read the files
directly.

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
MCP Bundle (`.mcpb`), which asks for the folder when it is installed. A bundle
holds the manifest and the release executable at `server/close-control`
(`server/close-control.exe` on Windows) and is packed with
`npx @anthropic-ai/mcpb pack`. Bundles are not yet attached to releases.
