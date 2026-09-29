from __future__ import annotations

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
