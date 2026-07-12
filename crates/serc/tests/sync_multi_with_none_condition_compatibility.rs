#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_sync_multi_with_none_condition_owners() {
    let source = r#"def model_shape(manager1, manager2, complete, cls, config, extra, finalize):
    with manager1, manager2:
        if complete:
            fields = cls.fields()
        else:
            fields = cls.rebuild()
        if config.get("extra_fields_behavior") == "allow" and extra is not None:
            finalize(extra)
        return fields


def dataclass_shape(manager1, manager2, cls, collect, decorators, finalize):
    with manager1, manager2:
        if cls:
            fields = cls.fields()
        else:
            fields = collect(cls)
        if decorators is None:
            decorators = finalize(cls)
        return fields, decorators


def no_try_model(manager1, manager2, config, extra, finalize):
    with manager1, manager2:
        if config.get("extra_fields_behavior") == "allow" and extra is not None:
            finalize(extra)


def no_try_dataclass(manager1, manager2, decorators, finalize):
    with manager1, manager2:
        if decorators is None:
            decorators = finalize()
    return decorators


def direct_return(manager1, manager2, config, extra):
    with manager1, manager2:
        if config.get("extra_fields_behavior") == "allow" and extra is not None:
            return extra
    return None
"#;
    common::assert_matches_cpython_marshal(
        source,
        "sync_multi_with_none_condition_compatibility.py",
    );
}
