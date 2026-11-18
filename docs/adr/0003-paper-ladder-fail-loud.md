# ADR-0003: Paper matching ladder never prints on a wrong size silently
Date: 2026-11-02. Status: Accepted.
Decision: exact form (tolerance 1.0 mm per side) -> custom DEVMODE size (Windows, verified via DC)
-> nearest larger form only if the rule's scale policy allows (centered no-scale or fit)
-> otherwise fail the job with a precise user-readable error and a log entry.
Implemented and unit-tested in print-core::paper on all platforms.
