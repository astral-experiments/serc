use crate::common;

const CONSUMERS: &str = "def first(value):\n    return value in {'case', 'unusual_marker'}\n\
                        def second(value):\n    'Different constants tuple.'\n    return value in {'case', 'unusual_marker'}\n";

#[test]
fn matches_cpython_match_soft_keyword_interning() {
    for seed in [
        "match value:\n    case 1: pass\n",
        "if False:\n    match value:\n        case 1: pass\n",
        "def seed(value):\n    match value:\n        case 1: pass\n",
        "if False:\n    def seed(value):\n        match value:\n            case 1: pass\n",
        "class Container:\n    match value:\n        case 1: pass\n",
        "async def seed(value):\n    match value:\n        case 1: pass\n",
    ] {
        for before in [false, true] {
            for prior_literal in [false, true] {
                let mut source = if prior_literal {
                    "prior = 'case'\n".to_string()
                } else {
                    String::new()
                };
                if before {
                    source.push_str(seed);
                    source.push_str(CONSUMERS);
                } else {
                    source.push_str(CONSUMERS);
                    source.push_str(seed);
                }
                common::assert_matches_cpython_marshal(&source, "match_soft_keyword.py");
            }
        }
    }
}

#[test]
fn matches_cpython_case_literals_without_match_statements() {
    for seed in [
        "",
        "prior = 'case'\n",
        "def seed():\n    return 'case'\n",
        "if False:\n    def seed():\n        return 'case'\n",
    ] {
        for before in [false, true] {
            let source = if before {
                seed.to_string() + CONSUMERS
            } else {
                CONSUMERS.to_string() + seed
            };
            common::assert_matches_cpython_marshal(&source, "case_literal_without_match.py");
        }
    }
}
