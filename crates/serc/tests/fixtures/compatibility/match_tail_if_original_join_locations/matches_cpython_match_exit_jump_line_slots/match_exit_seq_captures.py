def f(value, ready):
    try:
        match value:
            case (value, aid, sid):
                return (int(aid), int(sid))
            case (value, aid):
                return (int(aid), None)
    except ValueError:
        pass
    return None
