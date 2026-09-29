def call_then_dict(value):
    try:
        return primary(value)
    except ValueError as error:
        try:
            return secondary(value)
        except ValueError:
            try:
                return fallback(value)
            except ValueError:
                return {}


def dict_then_call(value):
    try:
        return primary(value)
    except ValueError as error:
        try:
            return secondary(value)
        except ValueError:
            try:
                return {"value": value}
            except ValueError:
                return fallback(value)
