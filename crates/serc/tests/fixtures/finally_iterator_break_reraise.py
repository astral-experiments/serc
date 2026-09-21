def ordinary_cleanup(items, manager):
    try:
        visit()
    finally:
        for item in items:
            with manager:
                item()
            break


def generator_cleanup(items, manager):
    try:
        yield 1
    finally:
        for item in items:
            with manager:
                item()
            break


async def async_generator_cleanup(items, manager):
    try:
        yield 1
    finally:
        for item in items:
            if item:
                with manager:
                    if item:
                        await item()
                    else:
                        item()
                break
