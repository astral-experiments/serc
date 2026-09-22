def function(kind, record, marker, cause, context, flag):
    try:
        try:
            raise Exception
        except kind:
            pass
    except TypeError:
        pass
    except Exception:
        record()
    try:
        try:
            raise Exception
        except (kind,):
            pass
    except TypeError:
        return
    except Exception:
        record()
