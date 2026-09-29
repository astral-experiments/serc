async def sample(rows, ready, sink):
    try:
        yield
    finally:
        sink(f"{sink([f'{key}:{value}' for key, value in rows])}{'yes' if ready else 'no'}")
