async def choose(fixture, enabled: bool, label: str | None, expected: str) -> None:
    primary = (await registry.resolve(Choices.PRIMARY)).value
    alternate = (await registry.resolve(Choices.ALTERNATE)).value
    request = Request(model='base', action='next', messages=[])
    pending: Future[Path | None] = Future()
    pending.set_result(None)
    settings = Mock()
    settings.option.return_value = True
    campaign = Campaign(
        model=alternate,
        name='trial',
        arm='alpha',
        rule=Rules.SPECIAL,
        override=primary,
        sampling=Sampling.MULTI,
        stream=Streams.DEFAULT,
        display=Display.ALLOW,
        treatment=Treatment.NORMAL,
        targeting=Targeting.SPECIAL,
    )
    check = AsyncMock(return_value=True)
    deferred = Mock(return_value=enabled)
    limit = Limit(name='cap', minutes=60, count=2)
    experiment = (
        Experiment(
            model=primary,
            exposure=False,
            name=label,
            arm='base' if label else None,
            rate=0,
            cap=None,
            window=None,
        )
        if label
        else Experiment.outside(primary)
    )
    with (
        patch('service.rate', return_value=Mock()),
        patch('service.settings', new=AsyncMock(return_value=settings)),
        patch('service.model', new=AsyncMock(return_value=experiment)),
        patch('service.contains', new=AsyncMock(return_value=False)),
        patch('service.agent', new=AsyncMock(return_value=False)),
        patch('service.adjusted', return_value=False),
        patch('service.variant', new=AsyncMock(return_value=campaign)),
        patch('service.candidate', new=AsyncMock(return_value=None)),
        patch('service.check', check),
        patch.object(Experiment, 'limit', new_callable=Property, return_value=limit),
        patch('service.deferred', deferred),
        patch('service.event'),
        patch('service.counter'),
    ):
        result = await resolve(
            user=fixture.user,
            request_id='request',
            body=request,
            model='base',
            agent=Mock(),
            session=Mock(),
            cache=Mock(),
            callsite='SELECT',
            pending=pending,
            event=Mock(),
        )
    deferred.assert_called_once_with(
        user=fixture.user,
        rule=Rules.SPECIAL,
        name=expected,
        targeting=Targeting.SPECIAL,
        sampling=Sampling.MULTI,
    )
    assert result.name == expected
    assert result.limit is limit
    if enabled:
        check.assert_not_awaited()
    else:
        check.assert_awaited_once()
        arguments = check.await_args
        assert arguments is not None
        assert arguments.args[3] is limit
        assert arguments.kwargs['dry'] is False
