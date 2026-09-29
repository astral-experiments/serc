def plain(value: left if gate else right) -> (left and right):
    return value

def protected(value: [candidate if gate else fallback
                      for candidate in values if keep(candidate)]):
    try:
        if outer:
            if left or right:
                consume(value)
            else:
                consume(None)
        else:
            if left or right:
                consume(None)
            else:
                consume(value)
    except ValueError:
        cleanup()
    return value
