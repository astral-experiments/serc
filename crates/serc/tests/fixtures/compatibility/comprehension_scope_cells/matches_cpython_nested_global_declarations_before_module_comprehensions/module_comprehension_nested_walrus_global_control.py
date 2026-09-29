def configure():
    global captured
    captured = None
result = [(captured, [(captured := value) for value in values], captured) for outer in groups]
after = captured
