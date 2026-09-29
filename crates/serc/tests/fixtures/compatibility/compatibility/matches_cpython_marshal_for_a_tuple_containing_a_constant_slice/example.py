constant = lambda value: value[:, 1]
dynamic = lambda value: value[::-1]
with_defaults = lambda value=1, other=2: value
string_slice = 'héllo'[1:4]
stepped_string_slice = 'abcdef'[1:5:2]
boolean_slice = 'abc'[False:True:True]
bytes_slice = b'prefix'[2:]
tuple_slice = (1, 2, 3)[1:]
negative_slice = 'abc'[:-1]
string_index = 'héllo'[1]
fstring_index = f'abc'[0]
bytes_index = b'prefix'[-1]
tuple_index = (1, 2, 3)[-2]
dynamic_list_index = [1, 2, 3][0]
member = item in [1, 2]
double_negation = not not item
5.0 ** 5.0
True ** True
False ** False
() << 0 ** 99999999999999999999999999
1 - 2
inverted = ~1
negative_zero = -0000
mixed = 1 + 2.5
assert (False, 'x')
assert (False,)
assert ()
assert True
packed = fn('User', **{'name': str})
unicode_keyword = fn(café=1)
huge = -999999999999999999999999999999999999999999
adjacent = ('' f'prefix {item}')
left if condition else right
'%s' % ('value',)
if (1, 2):
    pass
