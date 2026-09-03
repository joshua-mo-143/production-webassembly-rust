# Chapter 14 case study: operational runbook

Development-branch repository link:
<https://github.com/joshua-mo-143/production-webassembly-rust/tree/main/case-study/RUNBOOK.md>

This is the operational runbook for a deployment of the Chapter 14 secure
agent-tool runtime. It is a starting template, not a finished production
runbook: the case study has no alerting, dashboards, service-level objectives,
external telemetry sink, wall-clock deadline, or key-management process. Every
step below that depends on one of those is marked.

Fill in the owner and escalation fields for your deployment before the first
incident. A runbook with an unnamed owner is a document, not a procedure.

| Field | Value |
| --- | --- |
| Runbook version | 1 |
| Expects telemetry schema | Chapter 14 `Event` (stage, outcome, closed tool identity, detail) |
| Expects host | `ch14-host` against Wasmtime 48.0.0 |
| Owner | *unset* |
| Escalation contact | *unset* |
| Time to first action | *unset* |

The runbook version and the expected telemetry schema belong together. This
runtime records trusted classifications rather than attacker-controlled text,
so a runbook step that reads a field the host no longer emits fails silently
at exactly the wrong moment. Change them in the same commit.

## Capture before you inspect

Do this first, before classification and before any theory about cause.

Structured events live in an in-process overwrite ring with
`EVENT_CAPACITY = 64` and an `overwritten` counter. On a service under load the
ring has already wrapped by the time an operator arrives. Read the ring and its
overwrite count immediately: a non-zero `overwritten` tells you both that
evidence is gone and exactly how much.

The ring is process-local and lossy by design. If the process restarts, or the
workload is spread across processes, the ring is not an incident record. Forward
events to a protected external sink with retention and redaction before relying
on any investigation step below.

## Identify before you investigate

A friendly version string does not identify the code that ran. Record these
before forming a hypothesis:

| Identity | Where it comes from |
| --- | --- |
| Component name and SHA-256 digest | Signed manifest entry, verified at load |
| Manifest digest and signing identity | `target/ch14/manifest.json` and the separately supplied public key |
| Manifest format version | `MANIFEST_VERSION` (currently `1`) |
| WIT package, interface, and world | `book:secure-agent-tools@1.0.0` |
| Host release and source revision | Deployment metadata (*not present in the case study*) |
| Wasmtime version and engine configuration | Host build, currently Wasmtime 48.0.0 |
| Deployment generation and rollout cohort | Deployment metadata (*not present in the case study*) |

Two builds carrying the same human version can differ through dependencies,
compiler version, feature flags, or build inputs. The component digest is the
build identity; a release name may point to it but must not replace it.

## Classify

Operators see internal categories in structured events. The public `Display`
for every failure is always `tool request failed`, so the public message never
distinguishes anything. Do not build alerting on message text.

The host exposes twelve internal `RuntimeError` categories. Group them for
response:

| Group | Categories | First instinct |
| --- | --- | --- |
| Admission | `ArtefactRejected` | Release containment, not request rollback |
| Policy | `UnknownTool`, `DeniedTool`, `DeniedCapability`, `InvalidArguments` | Usually a client or grant problem |
| Request shape | `MalformedRequest` | Client or upstream serialization defect |
| Domain | `ComponentDeclaredFailure` | Expected typed failure; check whether the rate is new |
| Resource | `FuelExhausted`, `MemoryLimitDenied` | Workload change or abuse; do not raise limits yet |
| Guest defect | `GuestTrap` | Deterministic bug until proven otherwise; not retryable |
| Output | `InvalidOutput` | Host rejected the value *after* the component returned |
| Runtime | `RuntimeFailure` | Engine, linker, instantiation, or non-trap call failure |

`FuelExhausted` and `MemoryLimitDenied` are classified from the memory-limiter
flag and Wasmtime's typed `Trap`, not by parsing display strings. `InvalidOutput`
is never inferred from a trap, which matters during an incident: an invalid
output means the guest ran to completion and the host refused the result.

## Contain

Containment does not require understanding the cause.

- Pause the rollout or disable the affected tool.
- Drain the suspect generation; keep the last known-good generation loaded.
- Stop retries where the failure is deterministic. Retrying a `FuelExhausted`
  call with the same input and budget produces load, not recovery.
- Determine whether a host-owned external effect may already have started. The
  case study performs no external writes, so any deployment that adds them owns
  this question.

## Investigate

- Correlate the structured events captured above with the request that failed.
- Check consumed fuel against `CALL_FUEL` (`100_000`) and `INSTANTIATION_FUEL`
  (`3_000_000`), and memory growth against `MEMORY_LIMIT_BYTES` (4 MiB).
- Check input sizes against `MAX_REQUEST_BYTES` (2 048), `MAX_TEXT_CHARS` (256),
  and `MAX_DOCUMENT_BYTES` (16 KiB); check truncation against
  `MAX_OUTPUT_CHARS` (1 024).
- For admission failures, check the public key against
  `MAX_PUBLIC_KEY_BYTES` (4 KiB), the manifest against `MAX_MANIFEST_BYTES`
  (64 KiB), and each component against `MAX_COMPONENT_BYTES` (16 MiB). The
  loader rejects at `limit + 1` on an open-once read.
- Retrieve symbols or a sampled core dump from restricted storage only. Symbols
  expose function names, source paths, and internal types.

Fuel is deterministic work accounting, not a wall-clock deadline. A slow
blocking host operation will not exhaust fuel. If latency is the symptom, look
outside the store.

## Recover

- Route new work to the last known-good signed generation.
- Verify one known-safe request succeeds, and that a fresh store recovers after
  a classified failure. The runtime creates a fresh store and instance per call,
  so a trapped instance is never returned to a pool.
- Keep rollback protection enabled. Do not lower the minimum accepted
  generation to accept an older signed manifest.
- Confirm the event ring is being forwarded again if the sink was implicated.

## Escalate

| Condition | Escalate to |
| --- | --- |
| Evidence of native memory corruption or unauthorized host access | Security. Treat the embedding process and its host credentials as compromised |
| Signature verification failure that is not an explained publishing error | Security. Possible tampering |
| Secret or credential exposure in telemetry | Security |
| Process crash or unexplained `RuntimeFailure` | Runtime owners |
| Deterministic `GuestTrap` or `InvalidOutput` | Tool owner |

A guest trap is normally an application event. Evidence that the sandbox
boundary itself was crossed is a different severity entirely, and the response
is isolation, forensic preservation, secret rotation, and runtime patching
rather than a rollback.

## Per-alert procedures

Route distinct alerts to distinct procedures. A single "Wasm failure" entry
point is not actionable.

### Invalid-output burn

1. Capture the event ring and its overwrite count.
2. Compare stable and canary traffic by bounded generation.
3. Identify the failing tool and component digest.
4. Confirm the host rejected the value before any downstream use.
5. Pause or roll back the generation.
6. Preserve one sanitized failing input and the exact component bytes.
7. Add the output invariant as a test in the corrective release.

Stop and escalate if the rejected output contains data the tool should not have
been able to read. That is an exfiltration question, not an output-validation
question.

### Fuel-exhaustion burn

1. Separate a single tool regression from process-wide overload.
2. Compare consumed fuel, wall duration, queue time, and component digest.
3. Do **not** raise the fuel ceiling during the incident without a workload and
   abuse review. Raising it converts a visible failure into an invisible cost.
4. Disable or roll back deterministic offenders.
5. Verify a known-safe call and fresh-store recovery.

Stop and escalate if fuel consumption rose without a corresponding change in
input size or component digest.

### Artefact admission failure

1. Keep the candidate at load-only with zero traffic.
2. Check, in the loader's order: signature against the separately supplied
   public key, then manifest policy, then safe relative artefact paths, then
   SHA-256 digests, then WIT compatibility.
3. Distinguish a publishing error from possible tampering.
4. Preserve the rejected metadata and bytes under restricted access.
5. Never bypass admission by loading the component path directly.

Traffic should never reach a generation that failed admission, so this is
release containment rather than request rollback.

Note that a digest supplied by an untrusted manifest only establishes that the
file matches that same untrusted manifest. Signature verification against an
independently distributed public key is what makes the digest meaningful.

## What you can rehearse today

Several procedures above are exercisable against this repository, which is the
cheapest way to find out whether the runbook is wrong. Run from the repository
root:

```fish
cargo build --target wasm32-wasip2 \
 -p ch08-guest -p ch14-normalizer -p ch14-workspace-reader
cargo run -p ch14-host -- provision
cargo test -p ch14-host -- --ignored
cargo test --workspace
```

The five artefact-backed tests are `#[ignore]` and need the provisioned
manifest, so they run only under `--ignored`. The canonicalization test is an
ordinary unit test and runs under `cargo test --workspace`.

| Procedure | Exercised by | Command |
| --- | --- | --- |
| Resource classification and fresh-store recovery | `request_policy_capabilities_limits_and_recovery` | `--ignored` |
| Event-ring capture and overwrite counting | `telemetry_is_bounded_and_unknown_names_are_not_retained` | `--ignored` |
| Artefact admission, manifest tampering | `tampered_manifest_is_rejected` | `--ignored` |
| Artefact admission, component tampering | `tampered_component_is_rejected` | `--ignored` |
| Capability denial and recovery | `malicious_reader_cannot_read_sibling_and_healthy_reader_still_succeeds` | `--ignored`, also needs `ch08-guest` |
| Signing-byte canonicalization | `signing_bytes_follow_independently_specified_jcs_ordering_and_escaping` | `--workspace` |

`RuntimeFailure` has no artefact-backed rehearsal here. The fixture does not
induce an engine or linker failure, so treat that category as untested.

## Game day

Run the exercise with an application owner, a runtime owner, and a security
responder. Reveal evidence in stages rather than all at once.

A useful scenario: a canary generation increases its traffic share, the
invalid-output rate rises for one tool only, a burn-rate alert fires, the
rollout is paused, traces isolate a single component digest at the
output-validation stage, no host-owned write effect is confirmed, a signed
rollback to the previous generation is approved, and the failure rate recovers.

Ask the responder to produce the exact host, manifest, component, policy, and
runtime identities; the alert calculation and affected indicator; evidence that
the failures occurred after the component returned; whether any external effect
may have started; the authorized rollback record; the point at which user
impact stopped; and one hypothesis with one falsifying test.

Then alter it. Suppose a host callback may already have written to a database:
the rollback decision now needs an idempotency or reconciliation check. Alter it
again so the manifest signature fails during load-only admission: traffic never
reaches the candidate, and the response becomes release containment.

Record the result the way you record release tests: scenario, participating
roles, timestamps, evidence actually available, decisions made, recovery time,
and follow-up owner. A fast rollback with the wrong component digest is not a
pass. Neither is a recovery that depended on one engineer's local shell history.

Timeline exercises expose missing timestamps and identity fields quickly. If
the available telemetry can say that invalid output increased but cannot say
which immutable bytes caused it, the observability work is not finished.

## Caveats

- This runbook assumes controls the case study does not implement: alerting,
  dashboards, service-level objectives, an external telemetry sink, deployment
  generations, rollback protection, wall-clock deadlines, and production key
  management. See the operational caveats in
  [`README.md`](README.md) before treating any step as production-ready.
- The checked-in Ed25519 key pair is an RFC 8032 test vector and is marked
  **TEST ONLY**. Signature verification here proves nothing about who was
  authorized to publish a release.
- Filesystem containment, symlink, canonicalization, and path-policy behaviour
  is established for Linux only. Do not carry these steps to another platform
  without platform-specific tests.
- Every execution of this runbook is evidence about the runbook. When an
  exercise or a real incident exposes a missing field, update the telemetry
  schema, the alert, and this file together, and bump the runbook version.
