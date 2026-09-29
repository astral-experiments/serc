def loop_tail_control(value, items):
    for item in items:
        try:
            action(item)
        except:
            if True:
                pass
    consume(value)
