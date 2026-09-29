def uploader_shape(track, payload, count):
    if track.enabled:
        try:
            write(payload, track.endpoint)
            distribution("batch.cardinality", count)
        except UploadError:
            if not track.endpoint.startswith("/debugger/v1/diagnostics"):
                downgrade()
                write(payload, track.endpoint)
                distribution("batch.cardinality", count)
            else:
                raise
        except Exception:
            report(track)


def langchain_shape(manager, result):
    with manager:
        try:
            next(result)
        except Exception as error:
            if not isinstance(error, TimeoutError):
                assert False, f"Expected TimeoutError, got {error}"


def vision_shape():
    try:
        model = create_model()
        model = compile_model(model)
        value = model(input_value())
        print(f"compiled model output: {value.shape}")
    except RuntimeError:
        if platform == "win32":
            print("Successfully caught RuntimeError on win")
        else:
            raise


def dulwich_shape(path):
    try:
        unlink(path)
    except PermissionError:
        if platform == "win32":
            chmod(path, writable | readable)
            unlink(path)
        else:
            raise


def outside_with_control(result):
    try:
        next(result)
    except Exception as error:
        if not isinstance(error, TimeoutError):
            assert False, f"Expected TimeoutError, got {error}"


def explicit_else_control(manager, result):
    with manager:
        try:
            next(result)
        except Exception as error:
            if not isinstance(error, TimeoutError):
                assert False, f"Expected TimeoutError, got {error}"
            else:
                report(error)


def raise_instead_of_assert_control(manager, result):
    with manager:
        try:
            next(result)
        except Exception as error:
            if not isinstance(error, TimeoutError):
                raise AssertionError(error)
