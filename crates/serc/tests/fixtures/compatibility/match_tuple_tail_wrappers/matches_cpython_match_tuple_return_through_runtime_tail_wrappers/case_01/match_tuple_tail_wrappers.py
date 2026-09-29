def f(subject, outer, optional, inner, example, comment):
    match subject:
        case Kind():
            if outer:
                if optional:
                    if inner:
                        comment = f"{comment} yes"
                    else:
                        comment = "no"
            else:
                comment = "other"
                if optional:
                    comment = f"{comment} tail"
        case _:
            comment = "default"
    return example, comment
