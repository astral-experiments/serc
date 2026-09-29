def conditional_keyword(flag, value, other):
    return consume(value if flag else None, extra=other)

def inverted_keyword(flag, value, other):
    return consume(None if flag else value, extra=other)

def multiline_keyword(flag, value, other):
    return consume(value if flag else None,
                   extra=other)
