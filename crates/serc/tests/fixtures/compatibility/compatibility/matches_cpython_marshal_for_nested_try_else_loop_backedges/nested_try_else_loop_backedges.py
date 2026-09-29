def nested_try_else_loop(first, second):
    try:
        while True:
            try:
                outer_value = take_outer()
            except OuterError:
                break
            else:
                while True:
                    try:
                        inner_value = take_inner()
                    except InnerError:
                        break
                    else:
                        consume(inner_value)
                        if first:
                            consume(outer_value)
                            if second:
                                consume(second)
    except Exception:
        recover()
    finish()
