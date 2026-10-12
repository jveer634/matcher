# Matcher Documentation

- [Architecture & Relationship Diagrams](./architecture.md): Visual models of the sharded matching engine, request lifecycles, data structures, and protobuf translation layers.
- [Write-Ahead Log (WAL) & Durability](./wal/README.md): Simple Binary Encoding (SBE) persistence, zero-lock parallel snapshotting, and background pruning.
- [Extending Versions & Schema Evolution](./wal/extending_versions.md): Detailed guide on SBE framing, version extension, backward/forward compatibility, and safe rollouts.
