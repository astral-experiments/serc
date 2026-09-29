def choose(flag, primary, prefix, fallback):
    value = primary if flag else combine(prefix, None or fallback.value)
    consume(value)
