def run(value):
    try:
        perform()
    except Error:
        report('failed')
        return Result(loaded=False, version=None)
    finally:
        if value is not None and value.ready:
            value.close()
