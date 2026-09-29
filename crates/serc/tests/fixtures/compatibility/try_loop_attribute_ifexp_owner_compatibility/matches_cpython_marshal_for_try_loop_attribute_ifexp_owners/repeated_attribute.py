
class Error(Exception):
    pass


def repeated_attribute(items):
    results = []
    for index, item in enumerate(items, start=1):
        if not item:
            return None
        try:
            results.append(
                {
                    "name": item.name if item.name else f"item{index}",
                    "payload": item.payload(),
                }
            )
        except Error:
            return None
    return results
