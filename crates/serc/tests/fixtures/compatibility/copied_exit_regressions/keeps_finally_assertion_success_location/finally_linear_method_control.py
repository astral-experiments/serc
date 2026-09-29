def target(resource, registry):
    try:
        resource.start()
        assert registry.ready is True
    finally:
        resource.stop()
    assert registry.count == 1
    assert not resource.is_alive()
