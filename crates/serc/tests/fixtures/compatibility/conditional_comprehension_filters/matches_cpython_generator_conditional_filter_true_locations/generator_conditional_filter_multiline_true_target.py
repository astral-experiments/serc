def selected(items, enabled):
    return tuple(
        item
        for item in items
        if (
            item.ready
            if enabled
            else True
        )
    )
