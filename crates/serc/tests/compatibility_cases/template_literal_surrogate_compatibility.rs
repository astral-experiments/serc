use crate::common;

#[test]
fn matches_cpython_marshal_for_template_literal_surrogates() {
    let source = r#"def regular(first, second):
    return t"\ud83d{first}\udc0d{second}\udfff"

def replacement_order(value):
    return t"\ufffd\ud800{value}\udfff�"

def raw_then_regular(value):
    return tr"\ud800 �{value}" t"\udfff"

def regular_then_raw(value):
    return t"\ud800{value}" rt"\udfff �"

def raw_control(value):
    return tr"\ud800 �{value}\udfff"

def replacement_control(value):
    return t"\ufffd�{value}�\ufffd"

def adjacent_regular():
    return t"\ud800" t"\udfff"

def adjacent_raw_then_regular():
    return rt"\ud800 �" t"\udfff"

def adjacent_regular_then_raw():
    return t"\ud800" tr"\udfff �"

def debug_text(value):
    return t"\ud800{value=}\udfff"

def debug_escape_text():
    return t"\ud800{'\\udfff �'=}"

def escaped_backslash(value):
    return t"\\ud800�{value}\udfff"

def long_escape(value):
    return t"\U0000d800{value}\U0000dfff"

def paired_surrogates():
    return t"\ud800\udc00"

def scalar_control():
    return t"\U00010000"
"#;
    common::assert_matches_cpython_marshal(source, "template_literal_surrogates.py");
}
