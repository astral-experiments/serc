def target(manager, consume, finish):
    try:
        with manager as value:
            consume(value)
            return True, None
    except Exception as exc:
        return False, exc
    finally:
        finish()
