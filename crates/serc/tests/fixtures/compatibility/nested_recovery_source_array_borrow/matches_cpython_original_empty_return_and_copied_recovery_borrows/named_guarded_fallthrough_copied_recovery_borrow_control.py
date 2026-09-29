async def f(obj, reject, need_cleanup, note, cleanup):
    try:
        try:
            value = obj.read()
            note(value)
        except ValueError as error:
            note(error)
            if reject:
                note(error)
            value = None
        result = obj.dump()
        note(result)
        return result, value
    except RuntimeError:
        return None
    finally:
        if need_cleanup and obj is not None:
            note(obj)
            try:
                await cleanup(obj)
                note(obj)
            except ValueError as error:
                note(error)
                raise
