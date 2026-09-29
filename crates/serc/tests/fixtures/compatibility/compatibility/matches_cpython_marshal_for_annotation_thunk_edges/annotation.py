from typing import Annotated, List, TypedDict

()
value: int

def marker():
    pass

class Item(TypedDict):
    nodes: List[TypedDict('Node', {'name': str})]

class Metadata:
    value: Annotated[int, marker()]

def outer():
    from pathlib import Path

    class Visitor:
        def visit(self, value: Path) -> None:
            pass
