def chained_recovery(left, right, token, decode):
    if left.kind == right.kind == token:
        try:
            first = decode(left.value)
            second = decode(right.value)
        except ValueError:
            pass
        else:
            if first == second:
                right.value = left.value
