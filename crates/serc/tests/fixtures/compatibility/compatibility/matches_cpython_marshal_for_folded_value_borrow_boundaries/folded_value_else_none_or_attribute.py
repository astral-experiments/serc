def choose(flag, primary, fallback):
    value = primary if flag else None or fallback.value
    consume(value)
