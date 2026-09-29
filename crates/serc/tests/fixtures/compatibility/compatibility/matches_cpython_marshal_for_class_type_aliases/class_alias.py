from __future__ import annotations

class Container:
    Local = int
    type Alias = Local
    type Generic[T] = tuple[Local, T]
    type Factory[T] = lambda: (Local, T)
