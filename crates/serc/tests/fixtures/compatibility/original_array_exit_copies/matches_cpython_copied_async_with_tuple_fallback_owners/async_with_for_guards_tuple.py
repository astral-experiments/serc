async def probe(manager, existing):
    seen = set()
    left = []
    right = {}
    try:
        async with manager as stream:
            async for item in stream:
                name = item.name
                if not name.endswith("_entry"):
                    continue
                key = item.data.get("key")
                if not key:
                    continue
                right[key] = transform(name)
                if key in seen or key in existing:
                    continue
                value = load(key)
                if value is None:
                    continue
                left.append(value)
                seen.add(key)
    except Exception:
        report()
    return left, right
