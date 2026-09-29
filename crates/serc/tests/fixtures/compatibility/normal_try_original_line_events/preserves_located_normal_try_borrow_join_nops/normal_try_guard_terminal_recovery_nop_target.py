def checked_value(acquire, recover, finish):
    try:
        value = acquire()
        if value is None:
            raise ValueError('missing')
    except Exception as error:
        try:
            recover(error)
        except OSError:
            return None
        return False
    finish(value)
    return value
