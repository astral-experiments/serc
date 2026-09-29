def f(rows, missing):
    if missing:
        return [
            row
            for row in rows
            if (row.primary or (row.group.key if row.group else None)) is None
        ]
    return [
        row
        for row in rows
        if (row.primary or (row.group.key if row.group else None)) is not None
    ]
