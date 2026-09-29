@contextmanager
def resource_context() -> Iterator[OwnedResourceContext]:
    dependencies = load_dependencies()
    executable_path = resolve_executable_path()
    resource_name = f"temporary_resource_{unique_id().hex}"

    with TemporaryDirectory(prefix="temporary-resource-") as directory:
        path = Path(directory)
        executor = dependencies.executor(
            executable=str(executable_path),
            host="127.0.0.1",
            port=select_port(dependencies.get_port),
            datadir=str(path / "data"),
            unixsocketdir=str(path),
            logfile=str(path / "resource.log"),
            startparams="-w",
            dbname="default",
            user="default",
            password="",
            options="",
            postgres_options="-c shared_memory_type=mmap",
        )

        with executor:
            executor.wait_for_resource()
            resource_url = make_resource_url(
                executor,
                database=resource_name,
            )
            assert_safe_resource_url(resource_url)

            cleaner = dependencies.cleaner(
                user=str(executor.user),
                host=str(executor.host),
                port=int(executor.port),
                version=executor.version,
                dbname=resource_name,
                password=getattr(executor, "password", None),
            )
            cleaner.init()
            resource: Resource | None = None
            try:
                resource = acquire_resource(resource_url)
                yield OwnedResourceContext(
                    resource=resource,
                    executor=executor,
                )
            finally:
                prepare_cleanup()
                try:
                    if resource is not None:
                        resource.engine.dispose()
                finally:
                    cleaner.drop()
