# Packages — Formats

Changes to a format bump its version.

## 1. Package manifest (Draft)
One file at the root of a package declares everything it brings (D41). The loader reads it; the
core never does.

```yaml
name: work-modes
version: 0.1.0
state_namespace: work-modes          # its state keys are work-modes:* (projected from its events)
organs: [modes_organ]                # e.g. offers a `set_mode` capability
middleware:
  - { hook: context, scope: shared, entry: modes.annotate }   # attaches "mode: professional"
commands: [mode]
skills: []
rules:                               # standing rules shown in <self>, in this order
  - "In professional mode, keep replies short and formal."
knob_offsets:                        # bounded, temporary; recorded as disposition.updated
  talkativeness: { min: -0.2, max: 0.0 }
```
