from __future__ import annotations

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
