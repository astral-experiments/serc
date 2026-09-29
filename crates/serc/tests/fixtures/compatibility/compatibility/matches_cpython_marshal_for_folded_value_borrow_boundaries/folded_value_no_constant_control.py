def choose(flag, primary, fallback):
    value = primary if flag else fallback.value
    consume(value)
