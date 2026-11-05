# Decisions

Each entry: the decision, the alternatives, the reason.

## D1 — Licence: MIT
- Alternatives: Apache-2.0, MPL-2.0.
- Reason: The build plan's default; the most permissive option for a Marketplace extension.

## D2 — Repository visibility: private until first release
- Alternatives: public from day one.
- Reason: Visibility wasn't specified. Private is the reversible choice; switch with `gh repo edit --visibility public`.
