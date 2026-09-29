def probe(table, consume, finish):
    try:
        first, second = table.split(".", 1)
    except ValueError:
        parts = table.split(".")
        if len(parts) == 2:
            first, second = parts
        else:
            first, second = "", parts[0]
    consume(first, second)
    finish()
