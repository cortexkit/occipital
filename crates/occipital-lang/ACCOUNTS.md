# Resource accounting

`SharedAccount::chat()` belongs to one chat opening/measurement window;
`SharedAccount::app()` belongs to the host's concurrent rich UI lifetime.
Clone these handles into instances. Creating a replacement handle is **not**
eviction or Reset: it starts a new host lifetime. There is deliberately no
balance-reset operation. The internal `Accounts` nests an interface or reply
account inside those handles. All three are admitted before work. Pending
reservations release on drop; spent fuel and expanded preparation do not.

Invalid scope combinations and reservation mutations return typed errors without
changing balances. Parse settlement reserves any excess returned by the syntax
crate before reporting success. Work already covered by the original reservation
stays spent even if reserving the excess fails; the error identifies the scope
that refused the additional reservation. A unit-ceiling breach is returned after charging the actual work, not by
clamping it. Multiplication overflow returns a typed error and keeps the original
reservation charged. Final reply parsing follows the same fallible settlement
path. Finalizing a unit before retaining any allocations, or asking to shrink
above the current retained-byte reservation, returns a typed error.

The five work units have independent fuel ceilings (§9.1). Preparation and
conversion consume cumulative local/chat fuel and expansion; reply parsing
also consumes reply/chat fuel. Fallback and evaluation do not consume the
preparation queue's fuel. Parse reserves twice the bounded source-prefix byte
length before entering the syntax crate, then settles at twice the returned
`Document.parsed_bytes`; its unit ceiling remains 600,000. Streamed replies
protect 600,000 fuel for finalization and allow at most 1,400,000 provisional
work, returning the final reservation's unused portion after settlement.
Hosts must schedule visible rows in transcript order; a shared chat admits at
most two simultaneous preparation/conversion units.
The account implementation does not read the transcript or perform I/O.

## Retained allocation tariff

For every allocation, with byte counts `requested` (live payload including Rust
headers) and `unused_capacity`, charge:

```
ceil(requested / 16) * 16 + 64 + unused_capacity
```

Overflow refuses admission. Unused capacity is bytes, not elements, and must
not be omitted when a Vec/String has spare slots. Rust Arc counters and any
alignment padding are part of the requested payload, not the 64-byte malloc
allowance. Separate allocations each pay a separate header. Shared values
keep the reservation alive until their last retained descendant is dropped;
their §9.3 logical size is never substituted for their Rust allocation size.
The ownership ledger's Vec backing storage and Arc header are allocations too,
so the retained charges include them before either grows.

The granule is 16 bytes on all four supported targets. This covers the normal
16-byte malloc alignment/size classes on x86_64 Linux (glibc) and arm64 Darwin
(macOS, iOS device, and iOS simulator). The 64-byte fixed allowance conservatively
covers glibc's two-word chunk header, minimum-chunk rounding, and Darwin's
small/tiny/nano per-object metadata (including metadata stored out of line).
These are object-level charges, not a claim to measure RSS: allocator arenas,
guard pages, sanitizer overhead and a host's replacement allocator are not
bounded by a per-object malloc formula. A host using a different allocator
must report a larger requested size when its overhead exceeds this allowance.
The same constants on all targets keep decisions deterministic; no call to
`malloc_usable_size`, platform allocator API or target-dependent logical-size
heuristic is involved.

`allocation-counter` is a dev-dependency solely to measure requested bytes
on the current test thread without unsafe code in this crate. Tests isolate
each retained allocation, including Arc headers and Vec spare capacity, and
assert its requested bytes never exceed the tariff. These tests run on Linux;
the Apple targets are compile checked, not allocator/RSS benchmarked.

Raw snapshot bytes, decoded assets and measurement buffers reported by the
host reserve retained memory in every enclosing scope. They are **not** the
per-revision 32 MiB decoded-byte estimate: successful admission of that estimate
does not allocate anything and does not exempt subsequent host allocation
reports from retained-memory ceilings. Host allocation leases should outlive
the corresponding buffer and be dropped on cancellation or eviction.
