def retire_between_iterators_value(xs, ys, flag):
    for y in ys:
        try:
            for x in xs:
                if flag:
                    return x
        except Inner:
            recover()


def retire_between_iterators_literal(xs, ys, flag):
    for y in ys:
        try:
            for x in xs:
                if flag:
                    return None
        except Inner:
            recover()


def retain_outer_collector_value(xs, ys, flag):
    try:
        for x in xs:
            try:
                if flag:
                    return x
            except Inner:
                recover()
    except Outer:
        recover()


def retain_outer_collector_literal(xs, ys, flag):
    try:
        for x in xs:
            try:
                if flag:
                    return None
            except Inner:
                recover()
    except Outer:
        recover()
