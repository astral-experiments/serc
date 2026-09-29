def filtered_loop(values, predicate, selector, sink):
    for value in values:
        if predicate(value) if selector else True:
            sink(value)
