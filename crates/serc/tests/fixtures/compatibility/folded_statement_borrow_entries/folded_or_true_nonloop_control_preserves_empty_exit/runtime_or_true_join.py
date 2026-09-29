def function(flag, other, value, consume):
    if flag or True:
        consume(value)
    else:
        consume(other)
    consume(value)
    if flag:
        consume(other)
    consume(value)
