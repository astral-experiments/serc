use crate::common;

#[test]
fn postponed_surrogate_string_literals_match_cpython() {
    let source = r#"from __future__ import annotations

bare: '\ud800'
union: '\ud800' | None
adjacent: '�' '\ud800' '\udfff' '\ufffd' '\N{REPLACEMENT CHARACTER}'
raw_first: r'\ud800' '\udfff'
raw_last: '\ud800' R'\udfff'
unicode: u'\U0000D800'
unicode_upper: U'\ud800' u'\udfff'
quoted: '\"\ud800\''
ordinary: '\ufffd�'
nested: dict['\ud800', tuple[r'\udfff', '\udfff']]

def f(value: '\ud800' | None) -> list['\udfff']:
    pass

class C:
    value: '�\ud800'
"#;
    common::assert_matches_cpython_marshal(source, "postponed_surrogates.py");
}

#[test]
fn postponed_surrogate_interpolated_strings_match_cpython() {
    let source = r#"from __future__ import annotations

ftext: f'\ud800{x}\udfff'
fraw: rf'\ud800{x}' f'\udfff'
fexpr: f'{"\ud800"}'
fexpr_nested: list[f'{"\ud800"}']
fformat: f'{x:\ud800}'
freplacement: f'�\ud800{x}\ufffd\udfff'
fnamed: f'\N{REPLACEMENT CHARACTER}\ud800{x}\udfff'
ttext: t'\ud800{x}\udfff'
traw: rt'\ud800{x}' t'\udfff'
tformat: t'{x:\ud800}'
treplacement: t'�\ud800{x}\ufffd\udfff'
tnamed: t'\N{REPLACEMENT CHARACTER}\ud800{x}\udfff'
"#;
    common::assert_matches_cpython_marshal(source, "postponed_surrogate_interpolations.py");
}

#[test]
fn postponed_annotations_keep_surrogates_distinct_from_replacement_characters() {
    common::assert_executes(
        r#"from __future__ import annotations
value: '\ufffd\ud800' | None
raw: r'\ud800' '\udfff'
assert __annotations__['value'] == "'�\\ud800' | None"
assert __annotations__['raw'] == "'\\\\ud800\\udfff'"
print('ok')
"#,
        "ok\n",
    );
}
