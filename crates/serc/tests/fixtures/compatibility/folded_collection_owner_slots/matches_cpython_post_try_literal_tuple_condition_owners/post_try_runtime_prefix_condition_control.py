async def inspect_payload(payload, prefixes):
    try:
        text = payload.strip()
    except (TypeError, ValueError):
        return None
    if text.startswith(prefixes):
        return None
    return text
