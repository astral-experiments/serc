class WithFooter:
    with manager:
        def method(self): ...

class WildcardFooter:
    match value:
        case int(): ...
        case _:
            def method(self): ...

class FoldedBranchFooter:
    if ...:
        ...
    else:
        with manager:
            for value in values:
                def method(self): ...
