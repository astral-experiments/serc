def optimized_generator_controls(items):
    while all(item for item in items):
        yield items
    while any(item for item in items):
        yield items
    while tuple(item for item in items):
        yield items


def unary_not_control(item):
    while not should_continue(item):
        yield item
