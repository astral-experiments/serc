def probe(kind, payload, decode, use):
    match kind:
        case 'counters':
            try:
                counters: dict[str, int] = decode(payload)
            except Exception:
                pass
            else:
                use(counters)
        case 'date':
            use(payload)
        case _:
            pass
    return payload
