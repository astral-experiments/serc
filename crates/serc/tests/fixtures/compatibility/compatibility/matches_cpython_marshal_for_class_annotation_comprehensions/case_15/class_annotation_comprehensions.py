class C:
    type A = [(i, j) for i in (1, 2) for j in (i,) if j]
