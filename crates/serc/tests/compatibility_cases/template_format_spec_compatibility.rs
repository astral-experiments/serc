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
