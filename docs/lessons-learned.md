# Lessons learned

Hard-won lessons that cost real debugging time and are likely to
recur. See the "Lessons learned" section of
[`developer-guide.md`](developer-guide.md) for the bar and the shape
of an entry.

## Entries

<!--
Each entry follows this shape:

### <Short title — the failure mode, not the fix>

<The observed bug or surprising behaviour.>

*Cause*: <the underlying cause>.

*Rule*: <the prevention rule that future code should follow>.
-->

### New source types must emit the same probe shape as the existing ones

The built-in UUID-native sources (`DmiProductUuid`, `IoPlatformUuid`,
`WindowsMachineGuid`, `KenvSmbios`) emit a hyphenated UUID string as
their `Probe` value. `Wrap::Passthrough` relies on this: it parses
the raw value as a UUID and returns `None` if parsing fails. Sources
whose raw value isn't UUID-shaped (`MachineIdFile`'s hex, cloud
instance IDs, pod UIDs) must round-trip through `Wrap::UuidV5Namespaced`
to become UUIDs, which is the default.

When we added `AppSpecific<S>` ([#11]) we specifically chose to emit
a UUID string rather than the full 64-char hex HMAC, because:

- A novel probe shape silently breaks `Wrap::Passthrough` for that
  one source — callers relying on "preserve the source's own UUID"
  would get `None`.
- It would double-hash under the default `Wrap::UuidV5Namespaced`:
  SHA-1 over a SHA-256 digest. Harmless, but wasteful and confusing.

*Cause*: divergent probe shapes leak out of the `Source` abstraction
and change how every `Wrap` variant behaves for that source.

*Rule*: new source types — especially wrappers that compose with
other sources — should emit a UUID string whenever the output is
derived from a hash or a native UUID. If a caller needs raw
HMAC/SHA output, add a dedicated knob; don't change the default
probe shape.

[#11]: https://github.com/dekobon/host-identity/issues/11
