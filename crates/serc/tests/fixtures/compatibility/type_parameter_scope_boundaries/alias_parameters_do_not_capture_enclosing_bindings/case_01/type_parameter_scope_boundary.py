def outer():
    type Alias[T] = lambda: T
    T, = Alias.__type_params__
    return T, Alias
