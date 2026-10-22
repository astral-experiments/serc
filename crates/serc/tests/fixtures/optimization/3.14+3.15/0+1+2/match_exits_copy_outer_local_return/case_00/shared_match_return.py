def update(enabled, action, ready, item, value):
    if enabled:
        match action:
            case "update":
                if ready and value:
                    item.value = value
            case "clear":
                item.value = None
    else:
        fallback()
    return item
