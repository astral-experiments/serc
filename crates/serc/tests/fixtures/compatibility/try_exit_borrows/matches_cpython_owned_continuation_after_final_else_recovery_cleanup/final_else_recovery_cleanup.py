def f(flag, token_id, cache, decode, decode_bytes):
    decoded = 0
    try:
        decoded = decode(token_id)
    except Exception as outer_error:
        if flag:
            decoded = token_id
        else:
            try:
                decoded = decode_bytes(token_id)
            except Exception as inner_error:
                decoded = token_id
    cache[token_id] = decoded
    return decoded
