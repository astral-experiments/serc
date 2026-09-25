use crate::common;

#[test]
fn matches_cpython_for_postponed_integer_attributes() {
    let source = r#"from __future__ import annotations

decimal: (1).real
zero: (0).real
hexadecimal: (0x10).real
binary: (0b101).bit_length
octal: (0o777).real
large: (123456789123456789123456789).numerator
u64_max: (0xffff_ffff_ffff_ffff).real
large_hexadecimal: (0x1_0000_0000_0000_0000).real
above_u64: (0X1_0000_0000_0000_0001).real
large_binary: (0b1_00000000_00000000_00000000_00000000_00000000_00000000_00000000_00000000).real
large_octal: (0o2_000_000_000_000_000_000_000).real
union: (1).real | (2).imag
chained: (3).real.imag
subscript: tuple[(4).real, (5).imag]
formatted: f"{(6).real}"
format_spec: f"{(1).real:{(2).imag}}"
debug: f"{(1).real=}"
template: t"{(1).real:{(2).imag}}"
lambda_value: lambda: (7).real
kwargs: factory(**((1).real or (2).imag))
reordered_arguments: factory(key=(1).real, *(2).imag)

float_control: (1.5).real
imaginary_control: (2j).real
negative_control: (-1).real
string_control: "(1).real"

class Example:
    attribute: (8).real

def function(value: (9).real) -> (0x10).imag:
    pass
"#;

    common::assert_matches_cpython_marshal(source, "postponed_integer_attributes.py");
}

#[test]
fn matches_cpython_for_many_postponed_integer_attributes() {
    let annotation = std::iter::repeat_n("(1).real", 10_000)
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!("from __future__ import annotations\nvalue: ({annotation})\n");
    common::assert_matches_cpython_marshal(&source, "many_integer_attributes.py");
}
