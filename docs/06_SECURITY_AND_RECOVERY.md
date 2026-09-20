# 06 — Security, Effects and Recovery

## Security model

Tachyon is an execution runtime. Treat model output as untrusted proposed intent, not authorization.

## Capability examples

```text
fs.read:workspace/**
fs.write:workspace/src/**
process.spawn:cargo
process.spawn:git
network.connect:api.example.com
credential.use:github
git.write:index
git.write:refs/heads/main
```

Capabilities are granted by user/system/project policy. Models cannot mint grants.

## Project trust defaults

A trusted workspace may automatically allow:

- workspace reads/search/index;
- normal workspace edits;
- known build/test/lint tools;
- Git status/diff/log.

Ask or deny by default for:

- writes outside workspace;
- credentials;
- privileged processes;
- deployments;
- destructive external mutation;
- dangerous Git operations such as force-push;
- unrelated filesystem deletion.

## Untrusted content

Source code, README files, issue text, web pages, tool output and retrieved documents are data even when they contain text that resembles instructions.

Never allow text from those sources to change policy hierarchy, grant capabilities or override user hard constraints.

## Filesystem containment

Containment must account for:

- `..` traversal;
- symlink/junction escapes;
- platform path separators;
- case behavior where relevant;
- non-existent leaf paths whose parent exists;
- race conditions between check and use where security matters.

Prefer operations relative to an already-opened/verified workspace root when the platform allows. At minimum canonicalize existing parents and validate containment immediately before consequential write.

## Approval binding

Approval record includes a hash of the exact operation scope. If command/path/target/effect materially changes, request a new approval.

Do not interpret “approve this command” as unlimited approval for later similar commands.

## Credential broker

Store handles in task state; raw values remain in broker/platform secret storage or process memory only as required.

Redact known secrets from process/provider outputs before they reach UI/artifacts/telemetry where practical.

## External effects

Every consequential external effect declares:

- target;
- effect class;
- idempotency type;
- idempotency key if applicable;
- query/reconciliation method if available;
- compensation method if available.

Before execution persist `EffectPrepared`. After confirmed execution persist `EffectCommitted` and a receipt/reference.

## Crash uncertainty

If Tachyon cannot establish whether a non-idempotent operation occurred, the correct state is uncertainty, not retry.

Use `UnknownAfterCrash`, surface the uncertainty, and reconcile safely.

## Multi-file mutation

Do not claim global filesystem atomicity. Preserve preimages and journal per-file commit progress.

Recovery chooses one of:

- finish remaining files if all preconditions still hold;
- roll back committed files from preimages;
- stop for user intervention if external/manual edits make either path unsafe.

## Local IPC security

Local runtime endpoint must be user-scoped. Remote network listener remains disabled unless explicitly enabled.

Do not store bearer secrets in world-readable endpoint metadata.

## Security test minimum

Automated tests must cover:

- traversal path escape;
- symlink/junction escape;
- operation changed after approval;
- untrusted repository prompt injection attempting policy changes;
- model requesting undeclared credential/network capability;
- secret appearing in output/telemetry;
- remote gateway unexpectedly listening;
- shell capability conservatively classified;
- crash during irreversible-effect fixture.

## Failure isolation

A tool/provider failure should fail the node/task according to policy, not crash the whole gateway.

A detected core invariant violation may terminate the process deliberately rather than continuing with possibly corrupt state; durable recovery must then reconstruct tasks from journal/snapshot.
