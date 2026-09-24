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

#[test]
fn large_template_string_tuple_matches_cpython_stack_folding_limit() {
    use std::fmt::Write as _;

    for count in [0, 1, 28, 29, 30, 31, 40] {
        for kind in ["adjacent", "distinct", "parts", "debug", "formatted"] {
            let mut source = String::from("def work(value):\n    return (t\"");
            for index in 0..count {
                match kind {
                    "adjacent" => source.push_str("{value}"),
                    "distinct" => write!(source, "word{index}{{value}}").unwrap(),
                    "parts" => source.push_str("word{value}\"\n        t\""),
                    "debug" => source.push_str("{value=}"),
                    "formatted" => source.push_str("{value:>3}"),
                    _ => unreachable!(),
                }
            }
            if kind == "distinct" {
                source.push_str("last");
            }
            source.push_str("\")\n");
            common::assert_matches_cpython_marshal(
                &source,
                &format!("template_string_tuple_{kind}_{count}.py"),
            );
        }
    }
}

#[test]
fn executes_large_template_strings_in_order() {
    use std::fmt::Write as _;

    let mut source = String::from(
        "calls = []\ndef record(value):\n    calls.append(value)\n    return value\nresult = t\"",
    );
    for index in 0..31 {
        write!(source, "part{index}{{record({index})}}").unwrap();
    }
    source.push_str(
        "end\"\nprint(len(result.strings), len(result.interpolations))\nprint(result.strings[0], result.strings[-1])\nprint(calls == list(range(31)))\nprint([value.value for value in result.interpolations] == calls)\n",
    );
    common::assert_executes(&source, "32 31\npart0 end\nTrue\nTrue\n");
}
