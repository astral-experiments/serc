def sample(stop):
    if False:
        sink((5, 3), -1)
    try:
        if stop:
            return
    finally:
        sink((7, 11))
    for item in ['left', 'right']:
        sink(item)
    sink((5, 3), -0.5, -1)
