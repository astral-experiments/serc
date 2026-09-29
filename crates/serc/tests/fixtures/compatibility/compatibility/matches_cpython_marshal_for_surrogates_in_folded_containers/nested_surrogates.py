def tuple_argument(function):
    return function(("\uddf8",))

def iterated_values():
    for value in ["\ud800", ("\udfff",), "\ufffd"]:
        consume(value)
