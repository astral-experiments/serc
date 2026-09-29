def return_through_finally(items):
    for item in items:
        try:
            pass
        except Exception:
            return
        finally:
            consume(item)

def preserve_return_value():
    try:
        pass
        return produce()
    finally:
        consume()

def preserve_try_pass():
    try:
        pass
        return produce()
    except Exception:
        recover()
