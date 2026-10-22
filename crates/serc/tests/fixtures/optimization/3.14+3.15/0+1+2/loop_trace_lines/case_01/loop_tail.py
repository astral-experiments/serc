def process(ready, count):
    while ready():
        if count:
            count -= 1
        else:
            return
        pass
