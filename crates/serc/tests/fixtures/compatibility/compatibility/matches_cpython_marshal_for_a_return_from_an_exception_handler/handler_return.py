def f():
    value = source()
    try:
        raise ValueError
    except ValueError:
        return value

def nested():
    try:
        body()
    except ValueError as error:
        try:
            pass
        except TypeError:
            raise error

def nested_message():
    try:
        body()
    except ValueError as error:
        try:
            finish()
        except TypeError:
            log(f"error: {error}")
