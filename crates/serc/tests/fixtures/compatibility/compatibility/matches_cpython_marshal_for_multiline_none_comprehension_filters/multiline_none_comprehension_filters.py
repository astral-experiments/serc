def generator_name(items):
    return (
        item
        for item in items
        if (
            item
            is not None
        )
    )

def generator_named(items):
    return (
        value
        for item in items
        if (
            value := transform(item)
        ) is not None
    )

def list_call(items):
    return [
        item
        for item in items
        if (
            transform(item)
            is None
        )
    ]

def set_named(items):
    return {
        value
        for item in items
        if (
            value := transform(item)
        ) is not None
    }

def dict_named(items):
    return {
        value: item
        for item in items
        if (
            value := transform(item)
        ) is not None
    }

def same_line_generator(items):
    return (item for item in items if item is not None)

def same_line_list(items):
    return [item for item in items if transform(item) is None]
