def f(flag, token_id, cache, decode, decode_bytes):
    try:
        decoded = decode(token_id)
    except Exception:
        if flag:
            try:
                decoded = decode_bytes(token_id)
            except Exception:
                decoded = f"<tok:{token_id}>"
        else:
            decoded = f"<tok:{token_id}>"
        work()
    cache[token_id] = decoded
    return decoded
