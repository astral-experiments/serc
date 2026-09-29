def f(items, outer, inner, alternate):
    for item in items:
        match item:
            case 1:
                if outer:
                    if inner:
                        first()
                elif alternate:
                    second()
            case _:
                third()
