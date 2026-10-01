use super::guarded_loop_cleanup_exit;

fn admitted(source: &str) -> bool {
    let parsed = ruff_python_parser::parse_module(source).unwrap();
    guarded_loop_cleanup_exit(&parsed.syntax().body)
}

#[test]
fn guarded_loop_source_requires_a_completed_linear_loop_and_sole_none_exit() {
    let prefix = "error = None\nfor item in values:\n    try:\n        work(item)\n    except Exception as caught:\n        error = caught\n";
    let guard = "if error is not None:\n    raise error\n";
    assert!(admitted(&format!("{prefix}{guard}")));
    assert!(admitted(
        "for item in values:\n    with manager:\n        work(item)\nif error is not None:\n    raise Error(value) from cause\n"
    ));
    for tail in [
        "if error:\n    raise error\n",
        "if error is None:\n    raise error\n",
        "if flag and error is not None:\n    raise error\n",
        "if error is not None is flag:\n    raise error\n",
        "if error is not None:\n    work(value)\n    raise error\n",
        "if error is not None:\n    raise (await value)\n",
        "if error is not None:\n    raise (yield value)\n",
        "if error is not None:\n    raise Error(lambda x=(yield value): x)\n",
        "if error is not None:\n    return value\n",
        "if error is not None:\n    raise error\nelse:\n    work(value)\n",
    ] {
        assert!(!admitted(&format!("{prefix}{tail}")), "{tail}");
    }
    for prefix in [
        "for item in values:\n    work(item)\n",
        "for item in values:\n    try:\n        work(item)\n    except Exception:\n        work(value)\n    work(value)\n",
        "for item in values:\n    with manager:\n        break\n",
        "for item in values:\n    with manager:\n        continue\n",
        "for item in values:\n    with manager:\n        yield item\n",
        "for item in values:\n    with manager:\n        await work(item)\n",
        "for item in values:\n    try:\n        work(item)\n    finally:\n        work(value)\n",
        "for item in values:\n    with manager:\n        work(item)\nelse:\n    work(value)\n",
        "async for item in values:\n    with manager:\n        work(item)\n",
        "for item in work(lambda x=(yield value): x):\n    with manager:\n        work(item)\n",
        "for item in values:\n    with work(lambda x=(yield value): x):\n        work(item)\n",
    ] {
        assert!(!admitted(&format!("{prefix}{guard}")), "{prefix}");
    }
}
