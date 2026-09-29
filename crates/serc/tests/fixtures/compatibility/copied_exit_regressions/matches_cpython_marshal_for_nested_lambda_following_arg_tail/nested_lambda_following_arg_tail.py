key = lambda item: (
    score(item.value if ready else fallback(), item.kind)
    if item is not None
    else 0
)
