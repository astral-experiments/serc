from __future__ import annotations

flag = True

class Conditional:
    if flag:
        enabled: int
    else:
        disabled: bytes = b''

class NestedConditional:
    if flag:
        while flag:
            nested: str

class Unconditional:
    direct: int

class ConditionalWithoutAnnotation:
    if flag:
        direct = 1
