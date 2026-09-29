from __future__ import annotations

hexadecimal: 0x1_0000_0000_0000_0000
upper_hexadecimal: 0X1_0000_0000_0000_0001
binary: 0b1_00000000_00000000_00000000_00000000_00000000_00000000_00000000_00000000
octal: 0O2_000_000_000_000_000_000_000
positive: +0x1_0000_0000_0000_0000
negative: -0x1_0000_0000_0000_0000
inverted: ~0x1_0000_0000_0000_0000
negative_attribute: (-0x1_0000_0000_0000_0000).real
integer_attribute: (0x1_0000_0000_0000_0000).real
chained_attribute: (0x1_0000_0000_0000_0000).real.imag
binary_expression: 0x1_0000_0000_0000_0000 | 0x1_0000_0000_0000_0001
tuple_value: (0x1_0000_0000_0000_0000, 0x1_0000_0000_0000_0001)
subscript: tuple[(0x1_0000_0000_0000_0000, 0x1_0000_0000_0000_0001)]
call: factory(0x1_0000_0000_0000_0000, key=0x1_0000_0000_0000_0001)
kwargs: factory(**(0x1_0000_0000_0000_0000 or fallback))
formatted: f"{0x1_0000_0000_0000_0000}"
format_spec: f"{0x1_0000_0000_0000_0000:{0x1_0000_0000_0000_0001}}"
debug: f"{0x1_0000_0000_0000_0000=}"
template: t"{0x1_0000_0000_0000_0000:{0x1_0000_0000_0000_0001}}"

u64_max_control: 0xffff_ffff_ffff_ffff
small_control: 0x10
decimal_control: 18_446_744_073_709_551_616
float_control: 1.5
string_control: "0x1_0000_0000_0000_0000"

class Example:
    attribute: 0x1_0000_0000_0000_0000

def function(value: 0x1_0000_0000_0000_0000) -> 0x1_0000_0000_0000_0001:
    pass
