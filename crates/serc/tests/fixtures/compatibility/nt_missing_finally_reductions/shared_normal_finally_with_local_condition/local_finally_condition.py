def process(value, ready):
    try:
        store(value)
    except Exception as error:
        report(error)
    finally:
        try:
            if ready:
                finish(value)
        except Exception:
            pass
