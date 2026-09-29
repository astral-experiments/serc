async def collect(stream, check, report):
    values = []
    try:
        async for item in stream:
            if item < 0:
                raise ValueError(item)
            elif item:
                values.append(item)
                matched = any(value > 0 and check(value) for value in values)
                if matched:
                    break
        report(values)
        report(len(values))
        return values
    except ValueError:
        report("recovered")
    return []
