# ADR-0004: Queue delivery semantics
Date: 2026-11-02. Status: Accepted.
Decision: Durable SQLite (WAL) queue. Default semantics: at-least-once submission to the spooler
guarded by idempotency keys — a crash before spooler submission re-queues the job; a crash after
submission marks the job `printing` and, on recovery, it is NOT blindly reprinted (it moves to
`failed` with code RECOVERY_AMBIGUOUS unless the admin enables auto-reprint per profile).
This gives no-loss and no-duplicate in the provable cases, and an honest, visible choice in the
one ambiguous window (submitted to spooler, outcome unknown). Configurable per profile:
`recovery_mode = safe_no_reprint | auto_reprint`.
State machine: queued -> rendering -> spooling -> printing -> done | failed | canceled | dead_letter.
Invalid transitions are rejected in code and tested.
