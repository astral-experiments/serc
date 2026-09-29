def outer(T, values):
    type Alias = [T for item in values]
    return Alias, T
