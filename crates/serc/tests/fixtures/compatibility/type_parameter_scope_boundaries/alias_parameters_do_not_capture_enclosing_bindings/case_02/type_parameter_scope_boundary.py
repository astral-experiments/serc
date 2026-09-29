def outer():
    def generic[T, *Ts, **P]():
        type Alias[T2, *Ts2, **P2] = tuple[T, *Ts, P, T2, *Ts2, P2]
        return Alias
    T2, Ts2, P2 = generic().__type_params__
    return T2, Ts2, P2
