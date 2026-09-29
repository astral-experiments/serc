select = lambda item: (
    item if lower <= item.metadata.get("length", 0) <= upper else None
)
