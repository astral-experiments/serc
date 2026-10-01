from __future__ import annotations

value: int = 0
items = (1, 2)
live = [(lambda: z) for z in items]
assert [(lambda: a) for a in items]
assert {(lambda: b) for b in items}
assert {c: (lambda: c) for c in items}
assert [sum(d for other in items) for d in items]
