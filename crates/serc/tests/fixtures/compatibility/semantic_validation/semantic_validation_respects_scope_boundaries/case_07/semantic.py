from __future__ import annotations
def outer():
    def inner(x: (lambda: (yield 1))): pass
