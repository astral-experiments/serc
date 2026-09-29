def target(obj, initial, enabled, first, second):
    if first:
        if second:
            obj.note()
    try:
        value = initial
        if obj.enabled and enabled:
            value = obj.other
        obj.consume(value)
    except Error:
        obj.recover()
