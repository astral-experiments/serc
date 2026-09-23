# Even empty sequences must convert the count to a signed index-sized integer.
overflowing_repetitions = (
    lambda: '' * 9223372036854775808,
    lambda: 9223372036854775808 * '',
    lambda: b'' * 9223372036854775808,
    lambda: 9223372036854775808 * b'',
    lambda: () * 9223372036854775808,
    lambda: 9223372036854775808 * (),
    lambda: '' * 18446744073709551615,
    lambda: 18446744073709551615 * b'',
    lambda: () * 18446744073709551615,
    lambda: '' * 18446744073709551616,
    lambda: 18446744073709551616 * b'',
    lambda: () * 18446744073709551616,
    lambda: '' * -9223372036854775809,
    lambda: -9223372036854775809 * b'',
    lambda: () * -9223372036854775809,
)
for repeat in overflowing_repetitions:
    try:
        repeat()
    except OverflowError:
        pass
    else:
        raise AssertionError('repetition must preserve index overflow')

# The signed positive boundary remains valid for empty sequences.
assert (
    '' * 9223372036854775807,
    9223372036854775807 * b'',
    () * 9223372036854775807,
) == ('', b'', ())

# Nonempty negative repetitions remain subject to CPython's folding budget.
assert ('text' * -1, -1 * b'bytes', (1,) * -1) == ('', b'', ())
assert (
    'text' * -9223372036854775808,
    -9223372036854775808 * b'bytes',
    (1,) * -9223372036854775808,
) == ('', b'', ())
