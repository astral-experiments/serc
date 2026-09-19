use crate::common;

#[test]
fn matches_cpython_marshal_for_retained_finally_closure_receiver() {
    let source = r"def make_handler(pool, limiter):
    async def handle(request):
        session = await pool.get()
        try:
            await request.respond(serialize(session))
        except Error as error:
            try:
                await request.respond(make_error(error=error))
            except Error:
                report_failure()
        finally:
            pool.put_nowait(session)
            limiter.release()
    return handle
";
    common::assert_matches_cpython_marshal(source, "retained_finally_closure_receiver.py");
}
