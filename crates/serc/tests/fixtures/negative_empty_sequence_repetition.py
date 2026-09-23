# The copied exception exit makes folded constants follow graph admission order.
for repeat in ():
    try:
        repeat()
    except Exception:
        pass

# An earlier use of the empty literal must not acquire a later fold's dependencies.
prior = ('', 1)
empty = '' * -1
later = (2, 3)
reused_count = 'text' * -1
assert ('' * -1, -1 * '', b'' * -1, -1 * b'', () * -1, -1 * ()) == (
    '', '', b'', b'', (), (),
)
assert (
    '' * -9223372036854775808,
    -9223372036854775808 * '',
    b'' * -9223372036854775808,
    -9223372036854775808 * b'',
    () * -9223372036854775808,
    -9223372036854775808 * (),
) == ('', '', b'', b'', (), ())
assert (
    '' * (1 - 2),
    b'' * -True,
    ('x' * 0) * -1,
    -1 * (b'x' * 0),
    () * (-2 + 1),
) == ('', b'', '', b'', ())
assert ('text' * -1, -1 * b'bytes', (1,) * -1) == ('', b'', ())

# This count cannot become an empty result during folding.
overflow = lambda: '' * -9223372036854775809


def nested(flag):
    for repeat in ():
        try:
            repeat()
        except Exception:
            pass
    prior = ('', 1)
    if flag:
        empty = b'' * (1 - 2)
    else:
        empty = -1 * ()
    later = (2, 3)
    reused_count = 'text' * -1
    return prior, empty, later, reused_count


assert nested(True) == (('', 1), b'', (2, 3), '')
assert nested(False) == (('', 1), (), (2, 3), '')
