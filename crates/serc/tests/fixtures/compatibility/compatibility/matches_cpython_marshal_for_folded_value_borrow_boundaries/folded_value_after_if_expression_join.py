def choose(flag, enabled, tail):
    value = build(select() if flag else other(), enabled=False or enabled)
    consume(tail, value)
