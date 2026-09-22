shared = "shared-value"
strings = (
    "shared-value",
    "shared-value",
    "alpha",
    "beta",
    "gamma",
    "a much longer string that remains part of the constant table",
)
numbers = (None, True, False, 0, 1, -1, 255, 256, 65_536, 36_893_488_147_419_103_232, 1.5, -0.0, 2j)
bytes_values = (b"", b"shared", b"shared", bytes([0, 1, 2, 3]))
nested = ((1, 2), (1, 2), ("shared-value", (1, 2)), frozenset({1, 2, 3}))
membership = shared in {"alpha", "beta", "gamma", "shared-value"}
