# Data flow and network restrictions

## 1. Purpose and scope

Xero Ledger Review Gate is a **synthetic-only** local design demonstration for
reviewing Xero-shaped trial-balance fixtures. It does not process client exports
or establish that input came from Xero.

## 2. Zero-network contract
- The engine has no HTTP, socket or cloud telemetry libraries.
- It processes data locally in memory and makes no cloud LLM calls.
- It removes or pseudonymises account display names, entity names and identifying metadata before generating any artefact for a downstream agent.

## 3. Data flow model

```mermaid
flowchart LR
    Source["Synthetic Xero-Shaped Trial Balance CSV"] --> Ingestion["Schema & Hash Validation"]
    Ingestion --> Sandbox["In-Memory Decimal Math Engine"]
    Sandbox --> Split{"Artifact Splitter"}
    
    Split -->|Redacted Values Only| ModelArtifact["model-result.json<br/><i>(Bounded Review Values)</i>"]
    Split -->|Local Evidence Only| HumanArtifact["reviewer-evidence.json<br/><i>(For Human Review)</i>"]
    Split -->|Local Checksum Binding| Receipt["receipt.json<br/><i>(Unkeyed SHA-256 Digests)</i>"]
```

## 4. Receipt limitation

`receipt.json` is an adjacent, unkeyed local SHA-256 checksum binding. Anyone
who can replace the artefacts can replace the receipt. It detects mismatched
local files, but it does not prove authorship, source system, origin, time, or
immutability.
