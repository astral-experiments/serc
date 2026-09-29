def visit(rows, choose, make_iterator, consume):
    try:
        for row in rows:
            try:
                fast = choose(row)
                if not fast:
                    iterator = make_iterator(row)
                    while iterator.has_next():
                        value = iterator.next()
                        try:
                            consume(value)
                        except Exception:
                            continue
            except Exception:
                continue
    except RuntimeError:
        raise
