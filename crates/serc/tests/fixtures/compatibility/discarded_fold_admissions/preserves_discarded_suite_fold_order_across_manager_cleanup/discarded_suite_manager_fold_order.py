for token in tokens:
    with manager():
        if False:
            sink((5, 3), -1)
with manager():
    for item in ['left', 'right']:
        sink(item)
sink((5, 3), -0.5, -1)
