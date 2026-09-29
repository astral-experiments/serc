def nested(flag, inner, extra):
    if flag:
        if inner:
            first()
        else:
            second()
            if extra:
                third()
    else:
        fallback()
    after()
