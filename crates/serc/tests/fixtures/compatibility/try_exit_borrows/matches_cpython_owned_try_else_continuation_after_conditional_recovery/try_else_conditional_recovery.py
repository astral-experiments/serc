def f(flag, token_id, cache, decode, decode_bytes):
    decoded = 0
    try:
        decoded = decode(token_id)
    except Exception:
        if flag:
            try:
                decoded = decode_bytes(token_id)
            except Exception:
                decoded = token_id
        else:
            decoded = token_id
        work()
    else:
        decoded = None
    cache[token_id] = decoded
    return decoded
