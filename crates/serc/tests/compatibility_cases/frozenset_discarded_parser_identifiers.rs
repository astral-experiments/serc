use crate::common;
use crate::common::indent;

#[test]
fn matches_cpython_parser_identifiers_in_discarded_code() {
    for body in [
        "return ordinary_sentinel",
        "ordinary_sentinel = 1",
        "del ordinary_sentinel",
        "def nested(ordinary_sentinel): pass",
        "def nested(*, ordinary_sentinel): pass",
        "def nested(*ordinary_sentinel): pass",
        "return object_.ordinary_sentinel",
        "def ordinary_sentinel(): pass",
        "class ordinary_sentinel: pass",
        "match value:\n    case ordinary_sentinel: pass",
        "global ordinary_sentinel",
        "[ordinary_sentinel for ordinary_sentinel in ()]",
        "return other_name",
        "return 'ordinary_sentinel'",
    ] {
        let seed = format!("if False:\n    def seed():\n{}", indent(&indent(body)));
        let consumers = "def first(value):\n    return value in {'ordinary_sentinel', 'novel_marker'}\n\
                         def second(value):\n    'Distinct constants tuple.'\n    return value in {'ordinary_sentinel', 'novel_marker'}\n";
        for before in [false, true] {
            for private_name in [false, true] {
                let mut source = if before {
                    seed.clone() + consumers
                } else {
                    consumers.to_string() + &seed
                };
                if private_name {
                    source.push_str(
                        "class Trigger:\n    def method(self):\n        return self.__key\n",
                    );
                }
                common::assert_matches_cpython_marshal(&source, "discarded_parser_identifiers.py");
            }
        }
    }
}
