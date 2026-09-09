use crate::common;

#[test]
fn matches_cpython_dead_names_around_inlined_comprehensions() {
    for scope in ["module", "class", "function"] {
        for branch in ["if False", "while False"] {
            for dead in [
                "for item in values:\n    consume(item)",
                "consume(item)",
                "unused = [item for item in values]",
            ] {
                for comprehension in [
                    "[item for item in values]",
                    "{item: item for item in values}",
                    "[other for other in [item for item in values]]",
                ] {
                    let dead = format!("    {}\n", dead.replace('\n', "\n    "));
                    let mut source =
                        format!("{branch}:\n{dead}result = {comprehension}\nconsume(item)\n");
                    if scope != "module" {
                        let header = if scope == "class" {
                            "class Example:"
                        } else {
                            "def example(values):"
                        };
                        source = format!("{header}\n    {}", source.replace('\n', "\n    "));
                    }
                    common::assert_matches_cpython_marshal(
                        &source,
                        "dead_comprehension_name_scopes.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_walrus_cell_before_comprehension_save() {
    for initial in [
        "[value for item in items if (value := process(item))]",
        "[(value := process(item)) for item in items]",
        "{item: (value := process(item)) for item in items}",
    ] {
        for capture in ["", "    consume(lambda: value)\n"] {
            for later in [
                "[value for value in result]",
                "[lambda: value for value in result]",
                "[lambda: value for other in result for value in other]",
            ] {
                let source = format!(
                    "def example(items):\n    result = {initial}\n    intervening = 1\n{capture}    later = {later}\n    return result, later\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "walrus_cell_before_comprehension_save.py",
                );
            }
        }
    }
}
