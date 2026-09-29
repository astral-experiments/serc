def nested_handler_iterator_return(values):
    try:
        for value in values:
            try:
                try:
                    work(value)
                except Exception:
                    return str(value)
            except Exception:
                return str(value)
        return ""
    except Exception:
        return ""
