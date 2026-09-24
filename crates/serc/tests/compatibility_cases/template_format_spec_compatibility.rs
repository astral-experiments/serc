use crate::common;

#[test]
fn matches_cpython_marshal_for_surrogate_template_format_specs() {
    let source = r#"def regular(value):
    return t"{value:\ud800}"

def dynamic(value, width):
    return t"{value:\ufffd\ud800{width}\udfff�}"

def raw(value):
    return tr"{value:\ud800�}"

def raw_dynamic(value, width):
    return rt"{value:\ud800�{width}\udfff}"

def raw_then_regular(value):
    return tr"{value:\ud800�}" t"{value:\U0000dfff}"

def regular_then_raw(value, width):
    return t"{value:\ud800{width}}" tr"{value:\udfff�{width}}"

def nested_regular(value, echo):
    return t"{value:{echo:\ud800�}}"

def nested_raw(value, echo):
    return tr"{value:{echo:\ud800�}}"
"#;

    common::assert_matches_cpython_marshal(source, "surrogate_template_format_specs.py");
}

#[test]
fn executes_surrogate_template_format_specs() {
    let source = r#"class EchoSpec:
    def __format__(self, spec):
        return spec


value = 7
width = 4
echo = EchoSpec()
templates = (
    t"{value:\ud800}",
    t"{value:\ufffd\ud800{width}\udfff�}",
    tr"{value:\ud800�}",
    rt"{value:\ud800�{width}\udfff}",
    tr"{value:\ud800�}" t"{value:\U0000dfff}",
    t"{value:\ud800{width}}" tr"{value:\udfff�{width}}",
    t"{value:{echo:\ud800�}}",
    tr"{value:{echo:\ud800�}}",
)
for template in templates:
    print(ascii(tuple(item.format_spec for item in template.interpolations)))
"#;

    common::assert_executes(
        source,
        r"('\ud800',)
('\ufffd\ud8004\udfff\ufffd',)
('\\ud800\ufffd',)
('\\ud800\ufffd4\\udfff',)
('\\ud800\ufffd', '\udfff')
('\ud8004', '\\udfff\ufffd4')
('\ud800\ufffd',)
('\\ud800\ufffd',)
",
    );
}

const INTERPOLATION_SOURCE_TEXT: &str = r#"class Echo:
    def __format__(self, spec):
        return spec

a, b, c = 1, 2, 3
formatted = (
    f"{a != b=:}",
    f"{a!=b=:}",
    f"{a != b != c=:}",
    f"{a != b and b != c=:}",
    f"{a if b != c else a=:}",
    f"{(a != b)=:}",
    f"{a != b=!s:}",
    f"{Echo():{a != b=:}}",
)
templates = (
    t"{a != b != c:}",
    t"{a != b != c=:}",
    t"{a != b and b != c:}",
    t"{a if b != c else a:}",
    t"{(a != b):}",
    t"{a != b!s:}",
    t"{a != b=}",
    t"""{(a # : fake ! conversion
):}""",
    t"""{(a # : fake ! conversion
)!s}""",
    t"""{(a # ! fake : conversion
)!r:>4}""",
    t"""{(a # ! fake : conversion
)}""",
)
"#;

#[test]
fn matches_cpython_marshal_for_interpolation_source_text() {
    common::assert_matches_cpython_marshal(
        INTERPOLATION_SOURCE_TEXT,
        "interpolation_source_text.py",
    );
}

#[test]
fn executes_interpolation_source_text() {
    let source = format!(
        "{INTERPOLATION_SOURCE_TEXT}\n\
         for value in formatted:\n    print(repr(value))\n\
         for value in templates:\n    print(repr(value.strings), repr(value.interpolations[0].expression))\n"
    );
    common::assert_executes(
        &source,
        "'a True'\n\
         'aTrue'\n\
         'a != b True'\n\
         'a != b and b True'\n\
         'a if b 1'\n\
         '(a != b)=True'\n\
         'a != b=True'\n\
         'a True'\n\
         ('', '') 'a != b'\n\
         ('a != b ', '') 'a != b'\n\
         ('', '') 'a != b and b'\n\
         ('', '') 'a if b'\n\
         ('', '') '(a != b)'\n\
         ('', '') 'a != b'\n\
         ('a != b=', '') 'a != b'\n\
         ('', '') '(a \\n)'\n\
         ('', '') '(a \\n)'\n\
         ('', '') '(a \\n)'\n\
         ('', '') '(a \\n)'\n",
    );
}
