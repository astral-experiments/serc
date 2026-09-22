def operations(a, b, c):
    arithmetic = (a + b, a - b, a * b, a / b, a // b, a % b, a**b)
    bitwise = (a & b, a | b, a ^ b, a << b, a >> b, ~a)
    unary = (+a, -a, not a)
    comparisons = (a < b, a <= b, a == b, a != b, a > b, a >= b)
    membership = (a in c, a not in c, a is b, a is not b)
    chained = a < b <= c != a
    boolean = (a and b and c) or (a and not b) or c
    conditional = a if c else b
    return arithmetic, bitwise, unary, comparisons, membership, chained, boolean, conditional
