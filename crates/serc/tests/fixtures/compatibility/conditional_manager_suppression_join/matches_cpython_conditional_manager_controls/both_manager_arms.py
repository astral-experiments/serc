def convert(outer, left, right, choose, value):
    with outer:
        if choose:
            with left:
                return value
        else:
            with right:
                return value
