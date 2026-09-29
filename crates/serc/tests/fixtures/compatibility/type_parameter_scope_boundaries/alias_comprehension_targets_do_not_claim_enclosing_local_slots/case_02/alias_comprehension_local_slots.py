def outer():
    type Alias[T: [lambda: T for T in (T, [1])[1]]] = [lambda: T for T in T.__name__]
    func, = Alias.__value__
    T, = Alias.__type_params__
    return func, T
