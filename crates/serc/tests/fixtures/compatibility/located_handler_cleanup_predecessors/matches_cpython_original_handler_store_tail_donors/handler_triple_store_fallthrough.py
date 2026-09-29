def probe(table, consume, finish):
    try:
        first, second, third = table.split(".", 2)
    except ValueError:
        parts = table.split(".")
        if len(parts) == 3:
            first, second, third = parts
        elif len(parts) == 2:
            first, second, third = "", parts[0], parts[1]
        else:
            first, second, third = "", "", parts[0]
    consume(first, second, third)
    finish()
