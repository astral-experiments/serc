def ternary(a, b, c):
    return a if a else None, b, c

def pair(a, b):
    return a if a else None, b

def leading(a, b, c):
    return 1, a if a else None, b, c

def both_names(flag, a, b, c):
    return a if flag else a, b, c

def called(a, b, c):
    return consume(a if a else None, b, c)

def guarded(data):
    error_message = ""
    name = ""
    order = None
    if not data:
        error_message += "missing data"
    else:
        try:
            name = str(data["name"])
        except KeyError:
            error_message += "name has to be defined"
        try:
            order = int(str(data["order"]))
        except KeyError:
            error_message += "order has to be defined"
        except ValueError:
            error_message += "order has to be a number"
    return error_message if error_message else None, name, order

def quartet(a, b, c, d):
    return a if a else None, b, c, d

def inverted(a, b, c):
    return None if a else a, b, c
