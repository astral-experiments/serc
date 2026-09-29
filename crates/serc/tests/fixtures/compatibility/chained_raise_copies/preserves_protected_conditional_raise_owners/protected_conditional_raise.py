def function(first, second):
    try:
        if first:
            if second:
                error = ValueError if first else TypeError
            else:
                error = TypeError if first else ValueError
            raise error from None
        return None
    except Exception:
        recover()
