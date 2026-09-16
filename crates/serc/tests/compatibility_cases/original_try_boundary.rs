use crate::common;

#[test]
fn matches_cpython_original_boundary_compare_target() {
    let source = r#"async def scan(groups):
    for group in groups:
        if isinstance(group.parts, list):
            for item in group.parts:
                if item.kind == "picture":
                    payload: str = item.image.url

                    try:
                        if payload.startswith("inline:"):
                            prefix, body = payload.split(",", 1)
                            url = None
                        else:
                            prefix, body = "inline:picture", None
                            url = payload
                    except Exception as error:
                        raise ValueError("invalid image") from error

                    if body is not None:
                        consume(body)
"#;
    common::assert_matches_cpython_marshal(source, "original_boundary_compare_target.py");
}

#[test]
fn matches_cpython_original_boundary_primitive_control() {
    let source = r#"async def scan(groups):
    for group in groups:
        if isinstance(group.parts, list):
            for item in group.parts:
                if item.ready:
                    payload: str = item.image.url

                    try:
                        if payload.startswith("inline:"):
                            prefix, body = payload.split(",", 1)
                            url = None
                        else:
                            prefix, body = "inline:picture", None
                            url = payload
                    except Exception as error:
                        raise ValueError("invalid image") from error

                    if body is not None:
                        consume(body)
"#;
    common::assert_matches_cpython_marshal(source, "original_boundary_primitive_control.py");
}
