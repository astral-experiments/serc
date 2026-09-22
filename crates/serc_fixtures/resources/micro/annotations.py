from __future__ import annotations

value: int = 1
items: list[str]


type Pair[T] = tuple[T, T]
type Number = int | float


def identity[T](value: T, /) -> T:
    return value


def defaulted[T = str](value: T | None = None) -> T | None:
    return value


class Container[T: object]:
    value: T

    def get(self) -> T:
        return self.value
