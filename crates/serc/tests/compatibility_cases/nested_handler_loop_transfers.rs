use crate::common;

fn indent(source: &str) -> String {
    format!("    {}", source.replace('\n', "\n    "))
}

fn source(
    loop_header: &str,
    outer_handler: &str,
    inner_handler: &str,
    transfer: &str,
    loop_inside_handler: bool,
) -> String {
    let inner = format!("try:\n    g()\n{inner_handler}\n    {transfer}");
    let body = if loop_inside_handler {
        let inner_loop = format!("{loop_header}:\n{}", indent(&inner));
        format!("try:\n    f()\n{outer_handler}\n{}", indent(&inner_loop))
    } else {
        let outer = format!("try:\n    f()\n{outer_handler}\n{}", indent(&inner));
        format!("{loop_header}:\n{}", indent(&outer))
    };
    let prefix = if loop_header.starts_with("async ") {
        "async "
    } else {
        ""
    };
    format!(
        "{prefix}def probe(items, flag, f, g):\n{}\n    return 0\n",
        indent(&body)
    )
}

#[test]
fn matches_cpython_nested_handler_continue_cleanup() {
    for outer_handler in [
        "except:",
        "except ValueError:",
        "except ValueError as outer_error:",
    ] {
        for inner_handler in [
            "except:",
            "except KeyError:",
            "except KeyError as inner_error:",
        ] {
            common::assert_matches_cpython_marshal(
                &source(
                    "while flag",
                    outer_handler,
                    inner_handler,
                    "continue",
                    false,
                ),
                "nested_handler_continue.py",
            );
        }
    }
}

#[test]
fn matches_cpython_handler_cleanup_before_loop_transfers() {
    for loop_header in ["while flag", "for item in items", "async for item in items"] {
        for outer_handler in ["except ValueError:", "except ValueError as outer_error:"] {
            for transfer in ["continue", "break"] {
                common::assert_matches_cpython_marshal(
                    &source(
                        loop_header,
                        outer_handler,
                        "except KeyError:",
                        transfer,
                        false,
                    ),
                    "nested_handler_loop_transfer.py",
                );
            }
        }
    }
}

#[test]
fn retains_outer_handler_when_continue_targets_an_inner_loop() {
    for inner_handler in [
        "except:",
        "except KeyError:",
        "except KeyError as inner_error:",
    ] {
        common::assert_matches_cpython_marshal(
            &source(
                "while flag",
                "except ValueError as outer_error:",
                inner_handler,
                "continue",
                true,
            ),
            "handler_outside_continue_loop.py",
        );
    }
}
