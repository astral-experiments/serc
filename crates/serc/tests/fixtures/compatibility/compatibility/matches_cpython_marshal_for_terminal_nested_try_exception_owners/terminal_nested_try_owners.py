def terminal_nested_try(flag, items):
    try:
        if flag:
            try:
                for item in items:
                    consume(item)
            except Exception as error:
                report(error)
                report_done()
    except OuterError:
        recover()
    finish()


def nonterminal_nested_try(flag, items):
    try:
        if flag:
            try:
                for item in items:
                    consume(item)
            except Exception as error:
                report(error)
                report_done()
        finish_inside()
    except OuterError:
        recover()
    finish()
