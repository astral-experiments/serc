try:
    1 / 0
except ZeroDivisionError:
    pass

try:
    raise ValueError
except ValueError as error:
    print(error)

try:
    pass
except Exception:
    pass
finally:
    try:
        pass
    except BaseException as error:
        raise error

try:
    pass
except Exception as error:
    try:
        raise error
    except BaseException:
        pass

try:
    try:
        pass
    except BaseException as error:
        raise error
except Exception:
    pass
