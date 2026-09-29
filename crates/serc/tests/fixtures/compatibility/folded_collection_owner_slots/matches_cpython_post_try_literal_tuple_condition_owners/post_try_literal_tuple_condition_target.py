async def inspect_payload(payload):
    try:
        text = payload.strip()
    except (TypeError, ValueError):
        return None
    if text.startswith(("skip:", "empty:")):
        return None
    return text
