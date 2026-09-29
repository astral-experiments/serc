def string_return(primary, secondary, finish):
    try:
        return primary()
    except Exception:
        try:
            try:
                secondary()
            except Exception:
                return ""
            return finish()
        except Exception:
            return "fallback"


def bare_return(primary, secondary, finish):
    try:
        return primary()
    except Exception:
        try:
            try:
                secondary()
            except Exception:
                return
            return finish()
        except Exception:
            return None
